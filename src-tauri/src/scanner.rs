//! Junk scanner — resolves declarative rules and measures directory sizes
//! in parallel. Emits progress over a Tauri `Channel`; cancellation is
//! cooperative via a shared `AtomicBool`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use ignore::WalkBuilder;
use rayon::prelude::*;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::fsops;
use crate::rules::{self, CategoryRule, ScanTuning, TargetKind};
use crate::types::{CategoryResult, JunkItem, JunkScanResult, ScanProgress, JobPhase};

/// Statistics for one scanned directory.
#[derive(Debug, Default)]
struct DirStats {
    bytes: u64,
    files: u64,
}

/// Resolve a [`TargetRule`] into a list of existing filesystem paths.
fn resolve_targets(t: &rules::TargetRule, tuning: &ScanTuning, cancel: &Arc<AtomicBool>) -> Vec<PathBuf> {
    let base = match rules::expand_path(&t.dir) {
        Some(b) => b,
        None => return vec![],
    };
    if !base.exists() {
        return vec![];
    }

    match t.kind {
        TargetKind::Dir => vec![base],
        TargetKind::GlobChildren => {
            let pattern = match &t.pattern {
                Some(p) if !p.is_empty() => p.clone(),
                _ => return vec![],
            };
            glob_children(&base, &pattern)
        }
        TargetKind::MarkerDir => {
            let marker = match &t.pattern {
                Some(p) if !p.is_empty() => p.clone(),
                _ => return vec![],
            };
            let depth = t.max_depth.unwrap_or(6);
            find_marker_dirs(&base, &marker, depth, &tuning.marker_dir_exclude, cancel)
        }
    }
}

/// `ignore`-crate glob over the immediate children of `base` (one level deep
/// is the common case, but patterns like `*/Cache` need two levels — we walk
/// up to 4 levels which covers `Profile/Cache` layouts).
fn glob_children(base: &Path, pattern: &str) -> Vec<PathBuf> {
    use ignore::overrides::OverrideBuilder;

    let mut ob = OverrideBuilder::new(base);
    // The override matcher applies to the path *relative* to base.
    if ob.add(pattern).is_err() {
        return vec![];
    }
    let matcher = match ob.build() {
        Ok(m) => m,
        Err(_) => return vec![],
    };
    let trash_roots = fsops::trash_roots();

    WalkBuilder::new(base)
        .max_depth(Some(4))
        .hidden(false)
        .follow_links(false)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.depth() >= 1)
        .filter(|e| e.path().is_dir())
        // Trash roots are not temporary data. In particular, scanning `/tmp`
        // must not offer `/tmp/.Trash-<uid>` as a safe item to clean.
        .filter(|e| {
            let path = e.path();
            !trash_roots
                .iter()
                .any(|root| fsops::path_is_in_or_equal(path, root))
        })
        .filter(|e| matcher.matched(e.path(), e.path().is_dir()).is_whitelist())
        .map(|e| e.path().to_path_buf())
        .collect()
}

/// Breadth-first search for directories named `marker`, bounded by depth.
/// Pruned branches: symlinked dirs and names in `exclude`.
fn find_marker_dirs(
    base: &Path,
    marker: &str,
    max_depth: usize,
    exclude: &[String],
    cancel: &Arc<AtomicBool>,
) -> Vec<PathBuf> {
    let excluded: HashSet<&str> = exclude.iter().map(|s| s.as_str()).collect();
    let mut found = Vec::new();
    let mut frontier = vec![base.to_path_buf()];
    let mut depth = 0;

    while !frontier.is_empty() && depth < max_depth && !cancel.load(Ordering::Relaxed) {
        let mut next = Vec::new();
        for dir in frontier.drain(..) {
            let rd = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for entry in rd.flatten() {
                let ft = match entry.file_type() {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                if !ft.is_dir() || ft.is_symlink() {
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == marker {
                    found.push(entry.path());
                } else if !excluded.contains(name.as_ref()) {
                    next.push(entry.path());
                }
            }
        }
        frontier = next;
        depth += 1;
    }
    found
}

/// Measure a directory (or single file): total bytes + entry count.
/// Skips symlinks; unreadable entries count as 0 and don't fail the scan.
fn measure(path: &Path, counter: &AtomicU64, cancel: &Arc<AtomicBool>) -> DirStats {
    let mut stats = DirStats::default();
    if cancel.load(Ordering::Relaxed) {
        return stats;
    }
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return stats,
    };
    if meta.is_file() || (meta.file_type().is_symlink()) {
        if meta.is_file() {
            stats.bytes = meta.len();
            stats.files = 1;
        }
        return stats;
    }

    let walker = WalkBuilder::new(path)
        .hidden(false)
        .follow_links(false)
        .build();
    for e in walker {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let e = match e {
            Ok(e) => e,
            Err(_) => continue,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        if let Ok(m) = e.metadata() {
            if m.is_file() {
                stats.bytes += m.len();
                stats.files += 1;
            }
        }
    }
    stats
}

/// Run a junk scan for the given category ids (`None` = all).
///
/// Emits `ScanProgress` over `channel`. Returns the aggregated result.
pub fn run_junk_scan(
    category_ids: Option<&[String]>,
    rules_dir: Option<&Path>,
    channel: &Channel<ScanProgress>,
    cancel: Arc<AtomicBool>,
) -> JunkScanResult {
    let started = Instant::now();
    let emit = |phase: JobPhase, current: String, entries: u64, bytes: u64| {
        let _ = channel.send(ScanProgress {
            phase,
            current,
            entries_seen: entries,
            bytes_found: bytes,
            percent: None,
        });
    };

    let (rules, tuning) = match rules::load_rules(rules_dir) {
        Ok(r) => r,
        Err(e) => {
            emit(JobPhase::Error, e.to_string(), 0, 0);
            return JunkScanResult {
                categories: vec![],
                total_bytes: 0,
                duration_ms: started.elapsed().as_millis() as u64,
                cancelled: false,
            };
        }
    };

    let selected: Vec<CategoryRule> = match category_ids {
        Some(ids) => rules
            .into_iter()
            .filter(|c| ids.iter().any(|i| i == &c.id))
            .collect(),
        None => rules,
    };

    let entries_seen = Arc::new(AtomicU64::new(0));
    let bytes_found = Arc::new(AtomicU64::new(0));
    let mut categories = Vec::with_capacity(selected.len());
    let mut cancelled = false;

    for (ci, cat) in selected.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        emit(
            JobPhase::Scanning,
            cat.label.clone(),
            entries_seen.load(Ordering::Relaxed),
            bytes_found.load(Ordering::Relaxed),
        );

        // Resolve every target in this category to concrete paths.
        let mut paths: Vec<PathBuf> = Vec::new();
        for t in &cat.targets {
            if !rules::target_applies(t) {
                continue;
            }
            paths.extend(resolve_targets(t, &tuning, &cancel));
        }
        // Dedup: rules can overlap (e.g. glob `*` catches `pip` too).
        paths.sort();
        paths.dedup();
        // Drop paths nested inside another resolved path — deleting the
        // ancestor already removes them, and listing both would produce
        // "path does not exist" failures for the children.
        let mut roots: Vec<PathBuf> = Vec::with_capacity(paths.len());
        for p in paths {
            if roots.iter().any(|r| p.starts_with(r)) {
                continue;
            }
            roots.push(p);
        }
        let paths = roots;

        // Measure paths in parallel.
        let counter = Arc::new(AtomicU64::new(0));
        let counter2 = counter.clone();
        let cancel2 = cancel.clone();
        let measured: Vec<(PathBuf, DirStats)> = paths
            .par_iter()
            .map_init(
                || (),
                move |_, p| {
                    if cancel2.load(Ordering::Relaxed) {
                        return (p.clone(), DirStats::default());
                    }
                    let s = measure(p, &counter2, &cancel2);
                    (p.clone(), s)
                },
            )
            .collect();

        let entries = counter.fetch_add(0, Ordering::Relaxed);
        entries_seen.fetch_add(entries, Ordering::Relaxed);

        // Keep only paths that actually contain something.
        // For `glob_children` patterns like `*` on ~/.cache this drops
        // categories already claimed by more specific rules via overlap
        // filtering below.
        let items: Vec<JunkItem> = measured
            .into_iter()
            .filter(|(_, s)| s.bytes > 0)
            .map(|(p, s)| JunkItem {
                id: Uuid::new_v4().to_string(),
                path: p.to_string_lossy().into(),
                size_bytes: s.bytes,
                file_count: Some(s.files),
            })
            .collect();

        let total: u64 = items.iter().map(|i| i.size_bytes).sum();
        bytes_found.fetch_add(total, Ordering::Relaxed);
        emit(
            JobPhase::Scanning,
            format!("{}: {} items", cat.label, items.len()),
            entries_seen.load(Ordering::Relaxed),
            bytes_found.load(Ordering::Relaxed),
        );

        categories.push(CategoryResult {
            id: cat.id.clone(),
            label: cat.label.clone(),
            description: cat.description.clone(),
            risk: cat.risk.into(),
            total_bytes: total,
            items,
        });
        let _ = ci;
    }

    // Remove double-counted paths: if two categories claim the same path,
    // keep it in the *first* (more specific) category only. Also drop any
    // path nested inside a path already claimed — deleting the ancestor
    // removes it too.
    let mut claimed: Vec<PathBuf> = Vec::new();
    for c in categories.iter_mut() {
        c.items.retain(|i| {
            let p = PathBuf::from(&i.path);
            if claimed.iter().any(|r| p.starts_with(r)) {
                return false;
            }
            claimed.push(p);
            true
        });
        c.total_bytes = c.items.iter().map(|i| i.size_bytes).sum();
    }

    let total_bytes = categories.iter().map(|c| c.total_bytes).sum();
    emit(
        if cancelled { JobPhase::Cancelled } else { JobPhase::Done },
        format!("{} bytes", total_bytes),
        entries_seen.load(Ordering::Relaxed),
        total_bytes,
    );

    JunkScanResult {
        categories,
        total_bytes,
        duration_ms: started.elapsed().as_millis() as u64,
        cancelled,
    }
}
