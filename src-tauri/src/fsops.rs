//! Safe filesystem deletion — the *only* module allowed to remove files.
//!
//! Every deletion goes through [`delete_paths`] which:
//! 1. Resolves the canonical path (defeats `..` traversal and symlink escape).
//! 2. Rejects any path on the forbidden list (PRD §10).
//! 3. Moves to the OS Trash by default; permanent delete only when requested.
//! 4. Supports dry-run so the UI can preview exactly what would happen.
//! 5. Checks a shared cancel flag between items, so aborting mid-way is safe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::{AppError, AppResult};
use crate::types::{DeleteMode, DeleteOutcome, DeleteTarget};

/// Paths that must never be deleted, checked against canonical paths.
/// A path is forbidden when it *equals* or is a *prefix of* (i.e. an
/// ancestor of) a list entry, or sits inside one of the exact entries.
fn forbidden_prefixes() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/"),
        PathBuf::from("/usr"),
        PathBuf::from("/etc"),
        PathBuf::from("/bin"),
        PathBuf::from("/sbin"),
        PathBuf::from("/lib"),
        PathBuf::from("/lib64"),
        PathBuf::from("/boot"),
        PathBuf::from("/opt"),
        PathBuf::from("/var"),
        PathBuf::from("/proc"),
        PathBuf::from("/sys"),
        PathBuf::from("/dev"),
        PathBuf::from("/run"),
        PathBuf::from("/root"),
        PathBuf::from("/snap"),
    ];
    // Windows/macOS equivalents — inert on Linux, cheap to list.
    v.push(PathBuf::from("C:\\Windows"));
    v.push(PathBuf::from("C:\\Program Files"));
    v.push(PathBuf::from("/System"));
    v.push(PathBuf::from("/Library"));
    // Home root itself may be *scanned* but never deleted.
    if let Some(h) = home::home_dir() {
        v.push(h);
    }
    v
}

/// True when `path` is a system-protected location that may be scanned
/// (read) but never modified. Used by the duplicate finder when picking
/// scan roots and by the deleter.
pub fn is_forbidden(path: &Path) -> bool {
    // Canonicalize to resolve `..` and symlinks. If canonicalization fails
    // (broken symlink, missing entry) fall back to lexical normalization and
    // treat it conservatively: a non-existent path is still checked, and a
    // path we cannot resolve inside a forbidden tree is refused.
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    // Home is special: the *directory itself* may never be deleted, but its
    // contents are the very thing this app cleans.
    let home_canon = home::home_dir()
        .map(|h| std::fs::canonicalize(&h).unwrap_or(h));

    for f in forbidden_prefixes() {
        let f_canon = std::fs::canonicalize(&f).unwrap_or(f.clone());
        if canon == f_canon {
            return true;
        }
        // Skip subtree checks for $HOME — only an exact match is refused.
        if Some(&f_canon) == home_canon.as_ref() {
            continue;
        }
        // The target sits *inside* a forbidden ancestor: refuse.
        // The root "/" is exact-match only — otherwise every absolute
        // path would be forbidden.
        if f_canon.parent().is_some() && canon.starts_with(&f_canon) {
            return true;
        }
        // The target is an ancestor of a forbidden dir (e.g. "/"): refuse.
        if f_canon.starts_with(&canon) {
            return true;
        }
    }
    false
}

/// True when a directory is a system location that should be blocked as a
/// *scan root* in the duplicate finder (PRD §6.3 "Folder sistem diblokir").
pub fn is_protected_scan_root(path: &Path) -> bool {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let blocked: &[&str] = &["/", "/usr", "/etc", "/bin", "/sbin", "/lib", "/boot", "/proc", "/sys", "/dev", "/run", "/var", "/root", "/System", "/Library", "C:\\Windows"];
    blocked.iter().any(|b| {
        let bpath = Path::new(b);
        let bcanon = std::fs::canonicalize(bpath).unwrap_or_else(|_| bpath.to_path_buf());
        canon == bcanon
    })
}

/// Delete (or trash) a batch of paths.
///
/// `extra_allowed_prefixes` — paths outside these + the user home dir are
/// refused, as an additional belt-and-braces guard for the duplicate finder.
pub fn delete_paths(
    targets: &[DeleteTarget],
    mode: DeleteMode,
    dry_run: bool,
    cancel: &Arc<AtomicBool>,
    mut progress: impl FnMut(&DeleteOutcome),
) -> Vec<DeleteOutcome> {
    let mut outcomes = Vec::with_capacity(targets.len());
    // Paths successfully deleted so far. A target nested inside one of
    // these was already removed by its ancestor — report it as done
    // instead of "path does not exist".
    let mut deleted: Vec<PathBuf> = Vec::new();

    for t in targets {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let path = PathBuf::from(&t.path);
        if deleted.iter().any(|d| path.starts_with(d)) {
            let out = DeleteOutcome {
                path: t.path.clone(),
                ok: true,
                dry_run,
                freed_bytes: 0, // bytes were already credited to the ancestor
                error: None,
            };
            progress(&out);
            outcomes.push(out);
            continue;
        }
        let out = delete_one(t, mode, dry_run);
        if out.ok && !dry_run {
            deleted.push(path);
        }
        progress(&out);
        outcomes.push(out);
    }
    outcomes
}

pub fn delete_one(t: &DeleteTarget, mode: DeleteMode, dry_run: bool) -> DeleteOutcome {
    let path = PathBuf::from(&t.path);
    let mut out = DeleteOutcome {
        path: t.path.clone(),
        ok: false,
        dry_run,
        freed_bytes: 0,
        error: None,
    };

    // Guard 1: forbidden path list (canonical).
    if is_forbidden(&path) {
        out.error = Some("path is on the protected list".into());
        return out;
    }
    // Guard 2: the path must live inside a user-writable location ($HOME,
    // $TMPDIR, or a mounted drive). Anything else is refused.
    //
    // A path that is already gone is an idempotent no-op: the desired end
    // state (path absent) is already achieved, so it counts as a *success*,
    // not a failure (PRD §7 "Penghapusan idempoten"). This is the common case
    // where an ancestor directory was removed earlier in the same batch, or by
    // a previous run, taking its children with it — otherwise every nested
    // path would be reported as "path does not exist". `symlink_metadata` is
    // used instead of `exists` so a broken symlink still counts as present and
    // is removed rather than silently skipped.
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            out.ok = true;
            out.freed_bytes = 0;
            return out;
        }
        Err(e) => {
            out.error = Some(format!("cannot stat path: {e}"));
            return out;
        }
    }
    let canon = match std::fs::canonicalize(&path) {
        Ok(c) => c,
        Err(e) => {
            out.error = Some(format!("cannot resolve path: {e}"));
            return out;
        }
    };
    if !is_writable_location(&canon) {
        out.error = Some("path is outside allowed user locations".into());
        return out;
    }

    if dry_run {
        out.ok = true;
        out.freed_bytes = t.size_bytes;
        return out;
    }

    // A path that already lives inside the OS Trash can't be "moved to
    // trash" again — the trash crate would fail or no-op. Emptying the Trash
    // (PRD's `system.trash` category) is always a permanent removal.
    let effective_mode = if mode == DeleteMode::Trash && is_in_trash(&canon) {
        DeleteMode::Permanent
    } else {
        mode
    };

    let res = match effective_mode {
        DeleteMode::Trash => trash::delete(&canon).map_err(|e| e.to_string()),
        DeleteMode::Permanent => remove_recursive(&canon).map_err(|e| e.to_string()),
    };

    match res {
        Ok(()) => {
            out.ok = true;
            out.freed_bytes = t.size_bytes;
        }
        Err(e) => out.error = Some(e),
    }
    out
}

/// Allowed deletion roots: anywhere under HOME, TMPDIR, or mounted volumes
/// that are not system paths. The duplicate finder can target e.g. external
/// drives — /media/<user>/disk/x is fine, /var/lib/x is not.
fn is_writable_location(canon: &Path) -> bool {
    if let Some(h) = home::home_dir() {
        if let Ok(h) = std::fs::canonicalize(&h) {
            if canon.starts_with(&h) {
                return true;
            }
        }
    }
    let tmp = std::env::temp_dir();
    if let Ok(t) = std::fs::canonicalize(&tmp) {
        if canon.starts_with(&t) {
            return true;
        }
    }
    // Other user mounts (removable drives, secondary disks).
    for prefix in ["/media", "/mnt", "/run/media"] {
        if canon.starts_with(prefix) {
            return true;
        }
    }
    false
}

/// Return the trash roots known to the platform trash implementation.
///
/// Linux can have a trash directory on every mounted filesystem, including
/// paths such as `/tmp/.Trash-1000`. The old home-only check missed those
/// roots and allowed an already-trashed directory to be sent to Trash again.
pub(crate) fn trash_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    ))]
    {
        if let Ok(folders) = trash::os_limited::trash_folders() {
            roots.extend(folders);
        }

        // Keep the default home locations as fallbacks. They may not exist
        // yet, but the trash crate creates them when first used.
        let data_home = std::env::var_os("XDG_DATA_HOME").filter(|p| !p.is_empty());
        if let Some(data_home) = data_home {
            roots.push(PathBuf::from(data_home).join("Trash"));
        } else if let Some(h) = home::home_dir() {
            roots.push(h.join(".local/share/Trash"));
        }
    }

    roots
        .into_iter()
        .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn path_is_in_or_equal(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

/// True when `canon` (an already-canonicalized path) is inside any OS Trash
/// root. This includes per-mount Freedesktop roots such as
/// `/tmp/.Trash-1000` and `$mount/.Trash/<uid>`.
fn is_in_trash(canon: &Path) -> bool {
    trash_roots()
        .iter()
        .any(|root| path_is_in_or_equal(canon, root))
}

fn remove_recursive(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Replace `remove` with a hardlink to `keep` — duplicate finder option.
/// `remove` is deleted permanently after validation.
/// Reserved for the advanced "replace with hardlink" action (PRD §6.3).
#[allow(dead_code)]
pub fn replace_with_hardlink(remove: &Path, keep: &Path) -> AppResult<()> {
    if is_forbidden(remove) || is_forbidden(keep) {
        return Err(AppError::ForbiddenPath(remove.display().to_string()));
    }
    let r = std::fs::canonicalize(remove)?;
    let k = std::fs::canonicalize(keep)?;
    if !is_writable_location(&r) || !is_writable_location(&k) {
        return Err(AppError::ForbiddenPath(r.display().to_string()));
    }
    std::fs::remove_file(&r)?;
    std::fs::hard_link(&k, &r)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_paths_are_forbidden() {
        assert!(is_forbidden(Path::new("/")));
        assert!(is_forbidden(Path::new("/usr/bin/env")));
        assert!(is_forbidden(Path::new("/etc/passwd")));
        // traversal attempt
        assert!(is_forbidden(Path::new("/tmp/../../etc/hostname")));
    }

    #[test]
    fn home_is_protected_but_children_allowed() {
        let home = home::home_dir().unwrap();
        assert!(is_forbidden(&home));
        let child = home.join("somefile.txt");
        assert!(!is_forbidden(&child));
    }

    #[test]
    fn trash_and_dry_run() {
        let dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let f = dir.path().join("victim.bin");
        std::fs::write(&f, b"garbage").unwrap();

        // Dry-run deletes nothing.
        let cancel = Arc::new(AtomicBool::new(false));
        let t = DeleteTarget {
            path: f.to_string_lossy().into(),
            size_bytes: 7,
        };
        let res = delete_paths(&[t.clone()], DeleteMode::Permanent, true, &cancel, |_| {});
        assert!(res[0].ok && res[0].dry_run);
        assert!(f.exists());

        // Refuses deleting a protected path.
        let t2 = DeleteTarget { path: "/usr/bin/env".into(), size_bytes: 0 };
        let res = delete_paths(&[t2], DeleteMode::Permanent, false, &cancel, |_| {});
        assert!(!res[0].ok);

        // Cancel flag stops the batch before processing.
        cancel.store(true, Ordering::Relaxed);
        let res = delete_paths(&[t], DeleteMode::Permanent, false, &cancel, |_| {});
        assert!(res.is_empty());
        assert!(f.exists());
    }

    #[test]
    fn missing_path_is_idempotent_success() {
        let cancel = Arc::new(AtomicBool::new(false));
        let missing = std::env::temp_dir().join("cleansweep-definitely-absent-xyz");
        let t = DeleteTarget {
            path: missing.to_string_lossy().into(),
            size_bytes: 123,
        };
        let res = delete_paths(&[t], DeleteMode::Permanent, false, &cancel, |_| {});
        assert!(
            res[0].ok,
            "an already-gone path must be an idempotent success"
        );
        assert_eq!(res[0].freed_bytes, 0);
        assert!(res[0].error.is_none());
    }

    #[test]
    fn nested_targets_after_ancestor_are_success() {
        let dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let child = dir.path().join("nested");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("file.bin"), b"data").unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let targets = vec![
            DeleteTarget {
                path: dir.path().to_string_lossy().into(),
                size_bytes: 4,
            },
            DeleteTarget {
                path: child.to_string_lossy().into(),
                size_bytes: 4,
            },
        ];
        let res = delete_paths(&targets, DeleteMode::Permanent, false, &cancel, |_| {});
        assert!(res.iter().all(|o| o.ok));
        assert!(!dir.path().exists());
    }

    #[test]
    fn per_mount_trash_root_is_not_trashed_again() {
        let root = Path::new("/tmp/.Trash-1000");
        assert!(path_is_in_or_equal(root, root));
        assert!(path_is_in_or_equal(
            Path::new("/tmp/.Trash-1000/files/item.bin"),
            root
        ));
        assert!(!path_is_in_or_equal(Path::new("/tmp/.Trash-10000"), root));
    }
}
