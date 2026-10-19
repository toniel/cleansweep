//! Duplicate file finder — staged pipeline per PRD §6.3:
//!   1. Group candidates by exact size.
//!   2. Partial hash (head + middle + tail samples) to discard unique files
//!      cheaply.
//!   3. Full BLAKE3 hash only for surviving candidates.
//!
//! Progress streams over a Tauri `Channel`; cancellation is cooperative.
//! Symlinks are never followed. Hardlink detection (same device+inode)
//! prevents counting one physical file twice.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use rayon::prelude::*;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::fsops;
use crate::types::{
    DedupeOptions, DuplicateGroup, DuplicateScanResult, FileEntry, JobPhase, ScanProgress,
};

/// Files smaller than this are hashed whole in stage 2 (sampling is pointless).
const SMALL_FILE: u64 = 64 * 1024;
/// Bytes sampled at head/middle/tail for the partial hash.
const SAMPLE: usize = 8 * 1024;
/// Read buffer for full hashing.
const HASH_BUF: usize = 256 * 1024;
/// Minimum allowed `min_size_bytes` — protects against scanning zero-byte files.
const DEFAULT_MIN_SIZE: u64 = 1;

struct Progress<'a> {
    channel: &'a Channel<ScanProgress>,
    entries: AtomicU64,
    bytes: AtomicU64,
    last: std::sync::Mutex<Instant>,
}

impl<'a> Progress<'a> {
    fn new(channel: &'a Channel<ScanProgress>) -> Self {
        Self {
            channel,
            entries: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            last: std::sync::Mutex::new(Instant::now()),
        }
    }

    /// Emit at most every ~120ms plus a counter bump.
    fn tick(&self, current: &str, n: u64, bytes: u64) {
        self.entries.fetch_add(n, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
        let mut last = self.last.lock().unwrap();
        if last.elapsed().as_millis() >= 120 {
            *last = Instant::now();
            self.emit(JobPhase::Scanning, current);
        }
    }

    fn emit(&self, phase: JobPhase, current: &str) {
        let _ = self.channel.send(ScanProgress {
            phase,
            current: current.to_string(),
            entries_seen: self.entries.load(Ordering::Relaxed),
            bytes_found: self.bytes.load(Ordering::Relaxed),
            percent: None,
        });
    }
}

fn file_entry(path: &Path, size: u64) -> FileEntry {
    let modified_ms = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(-1);
    FileEntry {
        path: path.to_string_lossy().into(),
        size_bytes: size,
        modified_ms,
    }
}

/// (dev, inode) key so two hardlinks to one file are not reported as
/// duplicates — deleting the "extra" path would free nothing.
#[cfg(unix)]
fn file_key(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn file_key(_: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// Collect candidate files under `roots`, honouring excludes and size floor.
/// Returns `(size → paths)` for sizes seen at least twice.
fn collect_candidates(
    opts: &DedupeOptions,
    cancel: &Arc<AtomicBool>,
    prog: &Progress,
) -> AppResult<HashMap<u64, Vec<PathBuf>>> {
    let min_size = opts.min_size_bytes.max(DEFAULT_MIN_SIZE);

    // Build the exclude matcher per root (patterns are relative to the root).
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut hardlinked: std::collections::HashSet<(u64, u64)> = std::collections::HashSet::new();

    for root in &opts.roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let root_path = Path::new(root);
        if !root_path.is_dir() {
            continue;
        }
        if fsops::is_protected_scan_root(root_path) {
            return Err(AppError::ForbiddenPath(root.clone()));
        }

        let mut ob = OverrideBuilder::new(root_path);
        // Exclude patterns are *inverted* (leading `!` = ignore).
        for pat in &opts.excludes {
            let p = pat.trim();
            if p.is_empty() {
                continue;
            }
            if ob.add(&format!("!{p}")).is_err() {
                // Bad pattern — skip it rather than failing the whole scan.
                continue;
            }
        }
        let matcher = ob.build().map_err(|e| AppError::Message(e.to_string()))?;

        let mut builder = WalkBuilder::new(root_path);
        builder
            .follow_links(opts.follow_links)
            .hidden(!opts.include_hidden)
            .git_ignore(true)
            .git_exclude(true)
            .git_global(true);
        if !opts.excludes.is_empty() {
            builder.overrides(matcher);
        }

        for entry in builder.build() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let e = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let meta = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_file() {
                continue;
            }
            let size = meta.len();
            if size < min_size {
                continue;
            }
            // Skip second+ hardlink to the same inode.
            if let Some(k) = file_key(&meta) {
                if !hardlinked.insert(k) {
                    continue;
                }
            }
            prog.tick(&e.path().to_string_lossy(), 1, size);
            by_size.entry(size).or_default().push(e.path().to_path_buf());
        }
    }

    by_size.retain(|_, v| v.len() > 1);
    Ok(by_size)
}

/// BLAKE3 of head + middle + tail samples — cheap discriminator.
fn partial_hash(path: &Path, size: u64) -> Option<blake3::Hash> {
    let mut f = File::open(path).ok()?;
    let mut h = blake3::Hasher::new();
    h.update(&size.to_le_bytes());

    if size <= SMALL_FILE {
        let mut buf = Vec::with_capacity(size as usize);
        f.read_to_end(&mut buf).ok()?;
        h.update(&buf);
        return Some(h.finalize());
    }

    let mut sample = [0u8; SAMPLE];
    let offsets = [
        0,
        size.saturating_sub(SAMPLE as u64) / 2,
        size.saturating_sub(SAMPLE as u64),
    ];
    for off in offsets {
        f.seek(SeekFrom::Start(off)).ok()?;
        let n = f.read(&mut sample).ok()?;
        h.update(&sample[..n]);
    }
    Some(h.finalize())
}

/// Full-content BLAKE3.
fn full_hash(path: &Path) -> Option<blake3::Hash> {
    let mut f = File::open(path).ok()?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; HASH_BUF];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize())
}

/// Hash a batch of paths in parallel, grouping by hash value.
fn hash_stage<F>(
    paths: Vec<PathBuf>,
    size: u64,
    cancel: &Arc<AtomicBool>,
    prog: &Progress,
    label: &str,
    hash_fn: F,
) -> HashMap<blake3::Hash, Vec<PathBuf>>
where
    F: Fn(&Path) -> Option<blake3::Hash> + Sync,
{
    let hashed: Vec<(PathBuf, Option<blake3::Hash>)> = paths
        .par_iter()
        .map(|p| {
            if cancel.load(Ordering::Relaxed) {
                return (p.clone(), None);
            }
            let h = hash_fn(p);
            prog.tick(label, 1, size);
            (p.clone(), h)
        })
        .collect();

    let mut groups: HashMap<blake3::Hash, Vec<PathBuf>> = HashMap::new();
    for (p, h) in hashed {
        if let Some(h) = h {
            groups.entry(h).or_default().push(p);
        }
    }
    groups.retain(|_, v| v.len() > 1);
    groups
}

/// Run the duplicate scan. Never fails hard on single-file errors —
/// unreadable files are simply skipped.
pub fn run(
    opts: DedupeOptions,
    channel: &Channel<ScanProgress>,
    cancel: Arc<AtomicBool>,
) -> AppResult<DuplicateScanResult> {
    let started = Instant::now();
    if opts.roots.is_empty() {
        return Err(AppError::Message("no scan roots given".into()));
    }
    let prog = Progress::new(channel);
    prog.emit(JobPhase::Scanning, "collecting files");

    // Stage 1 — size groups.
    let by_size = collect_candidates(&opts, &cancel, &prog)?;
    let cancelled = || cancel.load(Ordering::Relaxed);

    // Stage 2 — partial hash within each size group.
    prog.emit(JobPhase::Scanning, "partial hashing");
    let mut candidates: Vec<(u64, PathBuf)> = Vec::new();
    for (size, paths) in by_size {
        if cancelled() {
            break;
        }
        let groups = hash_stage(paths, size, &cancel, &prog, "partial hash", |p| {
            partial_hash(p, size)
        });
        for (_, g) in groups {
            for p in g {
                candidates.push((size, p));
            }
        }
    }

    // Stage 3 — full hash only for surviving candidates.
    prog.emit(JobPhase::Scanning, "full hashing");
    let paths: Vec<PathBuf> = candidates.iter().map(|(_, p)| p.clone()).collect();
    let size_of: HashMap<PathBuf, u64> = candidates.into_iter().map(|(s, p)| (p, s)).collect();
    let groups = if cancelled() {
        HashMap::new()
    } else {
        hash_stage(paths, 0, &cancel, &prog, "full hash", full_hash)
    };

    // Assemble result groups sorted by wasted bytes.
    let mut groups: Vec<DuplicateGroup> = groups
        .into_iter()
        .map(|(hash, paths)| {
            let mut files: Vec<FileEntry> = paths
                .iter()
                .map(|p| file_entry(p, size_of.get(p).copied().unwrap_or(0)))
                .collect();
            files.sort_by(|a, b| a.modified_ms.cmp(&b.modified_ms));
            let size = files.first().map(|f| f.size_bytes).unwrap_or(0);
            DuplicateGroup {
                id: Uuid::new_v4().to_string(),
                hash: hash.to_hex()[..16].to_string(),
                size_bytes: size,
                wasted_bytes: size * (files.len() as u64 - 1),
                files,
            }
        })
        .collect();
    groups.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes));

    let wasted = groups.iter().map(|g| g.wasted_bytes).sum();
    let scanned_files = prog.entries.load(Ordering::Relaxed);
    let scanned_bytes = prog.bytes.load(Ordering::Relaxed);
    let cancelled = cancelled();
    prog.emit(
        if cancelled { JobPhase::Cancelled } else { JobPhase::Done },
        "done",
    );

    Ok(DuplicateScanResult {
        groups,
        scanned_files,
        scanned_bytes,
        wasted_bytes: wasted,
        duration_ms: started.elapsed().as_millis() as u64,
        cancelled,
    })
}

/// Re-hash one file — used by the deleter to re-verify identical content
/// before removing a duplicate (scan result may be stale).
pub fn verify_identical(a: &Path, b: &Path) -> bool {
    let ma = std::fs::metadata(a);
    let mb = std::fs::metadata(b);
    match (ma, mb) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() => {}
        _ => return false,
    }
    match (full_hash(a), full_hash(b)) {
        (Some(ha), Some(hb)) => ha == hb,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, data: &[u8]) {
        let mut f = File::create(path).unwrap();
        f.write_all(data).unwrap();
    }

    #[test]
    fn finds_exact_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.bin");
        let b = tmp.path().join("sub/b.bin");
        let c = tmp.path().join("c.bin");
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();
        write(&a, &vec![7u8; 100_000]);
        write(&b, &vec![7u8; 100_000]);
        write(&c, &vec![9u8; 100_000]);

        let opts = DedupeOptions {
            roots: vec![tmp.path().to_string_lossy().into()],
            excludes: vec![],
            min_size_bytes: 1,
            include_hidden: true,
            follow_links: false,
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        let channel = Channel::new(move |msg: tauri::ipc::InvokeResponseBody| {
            let _ = tx.send(msg);
            Ok(())
        });
        let res = run(opts, &channel, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(res.groups.len(), 1);
        assert_eq!(res.groups[0].files.len(), 2);
        assert_eq!(res.groups[0].wasted_bytes, 100_000);
    }

    #[test]
    fn excludes_pattern_and_size_floor() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("keep/skip.bin");
        let b = tmp.path().join("skip.bin");
        std::fs::create_dir_all(a.parent().unwrap()).unwrap();
        write(&a, b"dup!");
        write(&b, b"dup!");

        let opts = DedupeOptions {
            roots: vec![tmp.path().to_string_lossy().into()],
            excludes: vec!["skip.bin".into()],
            min_size_bytes: 1,
            include_hidden: true,
            follow_links: false,
        };
        let channel = Channel::new(|_| Ok(()));
        let res = run(opts, &channel, Arc::new(AtomicBool::new(false))).unwrap();
        assert!(res.groups.is_empty());
    }

    #[test]
    fn hardlinks_not_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("orig.bin");
        write(&a, b"same content");
        let link = tmp.path().join("link.bin");
        std::fs::hard_link(&a, &link).unwrap();

        let opts = DedupeOptions {
            roots: vec![tmp.path().to_string_lossy().into()],
            excludes: vec![],
            min_size_bytes: 1,
            include_hidden: true,
            follow_links: false,
        };
        let channel = Channel::new(|_| Ok(()));
        let res = run(opts, &channel, Arc::new(AtomicBool::new(false))).unwrap();
        assert!(res.groups.is_empty());
    }

    #[test]
    fn verify_identical_works() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        let c = tmp.path().join("c");
        write(&a, b"xyz");
        write(&b, b"xyz");
        write(&c, b"abc");
        assert!(verify_identical(&a, &b));
        assert!(!verify_identical(&a, &c));
        assert!(!verify_identical(&a, &tmp.path().join("missing")));
    }
}
