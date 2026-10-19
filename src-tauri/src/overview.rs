//! Disk / mount overview for the dashboard — thin wrapper over `sysinfo`.

use crate::types::DriveInfo;

/// List mounted disks with usage, filtering out pseudo-filesystems that
/// only confuse the "reclaimable space" summary.
pub fn list_drives() -> Vec<DriveInfo> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|d| {
            // Skip pseudo FS (snap loops, containers, tmpfs).
            let fs = d.file_system().to_string_lossy().to_lowercase();
            let total = d.total_space();
            total > 0
                && !matches!(
                    fs.as_str(),
                    "tmpfs" | "devtmpfs" | "squashfs" | "overlay" | "proc" | "sysfs" | "devpts" | "cgroup2" | "ramfs"
                )
        })
        .map(|d| {
            let total = d.total_space();
            let free = d.available_space();
            DriveInfo {
                name: d.name().to_string_lossy().into(),
                mount_point: d.mount_point().to_string_lossy().into(),
                kind: format!("{:?}", d.kind()),
                fs_type: d.file_system().to_string_lossy().into(),
                total_bytes: total,
                used_bytes: total.saturating_sub(free),
                free_bytes: free,
            }
        })
        .collect()
}
