//! Shared types used across modules and serialized to the frontend.

use serde::{Deserialize, Serialize};

/// Risk level of a junk category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Pure cache/temporary data — always safe to remove.
    Safe,
    /// Data loss is possible (trash contents, logs, Docker volumes).
    Caution,
}

/// Current lifecycle phase of a background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobPhase {
    Scanning,
    Deleting,
    Done,
    Cancelled,
    Error,
}

/// Progress event payload emitted over a Tauri channel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub phase: JobPhase,
    /// Human readable description of the current activity.
    pub current: String,
    /// Number of entries inspected so far.
    pub entries_seen: u64,
    /// Bytes found so far.
    pub bytes_found: u64,
    /// Optional percent (0-100) when the total is known.
    pub percent: Option<f32>,
}

/// A single cleanable file/directory found by the junk scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkItem {
    pub id: String,
    pub path: String,
    pub size_bytes: u64,
    /// None if the path vanished after measuring.
    pub file_count: Option<u64>,
}

/// Result for one junk category.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryResult {
    pub id: String,
    pub label: String,
    pub description: String,
    pub risk: RiskLevel,
    pub total_bytes: u64,
    pub items: Vec<JunkItem>,
}

/// Overall scan result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JunkScanResult {
    pub categories: Vec<CategoryResult>,
    pub total_bytes: u64,
    pub duration_ms: u64,
    pub cancelled: bool,
}

/// A path the caller asked to delete, with its reported size.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTarget {
    pub path: String,
    pub size_bytes: u64,
}

/// Deletion mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeleteMode {
    Trash,
    Permanent,
}

/// Per-path outcome of a delete request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    pub path: String,
    pub ok: bool,
    /// True when the operation was a dry-run only.
    pub dry_run: bool,
    pub freed_bytes: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    pub outcomes: Vec<DeleteOutcome>,
    pub freed_bytes: u64,
    pub cancelled: bool,
}

/// Disk usage summary for one mount point.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub name: String,
    pub mount_point: String,
    pub kind: String,
    pub fs_type: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
}

/// A record of a completed cleanup, persisted locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    /// RFC3339 timestamp.
    pub at: String,
    /// e.g. "junk", "docker", "duplicates"
    pub category: String,
    pub freed_bytes: u64,
    pub item_count: u64,
    /// "trash" | "permanent" | "docker"
    pub mode: String,
}

// ---------------------------------------------------------------------------
// Docker
// ---------------------------------------------------------------------------

/// Docker daemon availability and version info.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerStatus {
    pub available: bool,
    pub version: Option<String>,
    pub api_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerImageInfo {
    pub id: String,
    /// Repo tags; `["<none>:<none>"]` for dangling images.
    pub tags: Vec<String>,
    pub size_bytes: u64,
    /// Unix epoch seconds.
    pub created: i64,
    pub dangling: bool,
    /// Names of containers referencing this image.
    pub used_by: Vec<String>,
    /// True when at least one referencing container is running.
    pub in_use_running: bool,
    /// Pinned by the user — never suggested for deletion.
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    /// Image ID the container was created from.
    pub image_id: String,
    /// "running" | "exited" | "created" | ...
    pub state: String,
    pub status: String,
    pub created: i64,
    /// Writable-layer size when reported by the daemon.
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerVolumeInfo {
    pub name: String,
    pub driver: String,
    /// Reference count from `docker system df`.
    pub in_use: bool,
    pub size_bytes: Option<u64>,
    pub created_at: Option<String>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerNetworkInfo {
    pub id: String,
    pub name: String,
    pub driver: String,
    /// Containers currently attached.
    pub in_use: bool,
    /// Built-in networks (bridge/host/none) can never be removed.
    pub builtin: bool,
}

/// Per-record build cache entry — reserved for the detailed BuildKit listing
/// (currently only the aggregate from `system df` is surfaced).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerBuildCacheInfo {
    pub id: String,
    pub description: String,
    pub size_bytes: u64,
    pub in_use: bool,
    pub shared: bool,
}

/// Everything the Docker tab needs in one round-trip.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerOverview {
    pub status: DockerStatus,
    pub images: Vec<DockerImageInfo>,
    pub containers: Vec<DockerContainerInfo>,
    pub volumes: Vec<DockerVolumeInfo>,
    pub networks: Vec<DockerNetworkInfo>,
    /// Aggregated build-cache usage from `system df` (individual records can
    /// be numerous; the UI shows the total and offers a single prune action).
    pub build_cache_records: u64,
    pub build_cache_bytes: u64,
    /// Lower bound of reclaimable space.
    pub reclaimable_bytes: u64,
}

/// Docker resource kind for delete requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerResourceKind {
    Image,
    Container,
    Volume,
    Network,
    /// Build cache is pruned wholesale (the daemon only exposes `build/prune`).
    BuildCache,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerDeleteRequest {
    pub kind: DockerResourceKind,
    pub ids: Vec<String>,
    #[serde(default)]
    pub dry_run: bool,
}

// ---------------------------------------------------------------------------
// Duplicate finder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub size_bytes: u64,
    /// Unix epoch milliseconds; -1 when unknown.
    pub modified_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub id: String,
    /// Short full-hash fingerprint (first 16 hex chars).
    pub hash: String,
    pub size_bytes: u64,
    /// `size * (count - 1)` — space freed by removing all but one copy.
    pub wasted_bytes: u64,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateScanResult {
    pub groups: Vec<DuplicateGroup>,
    pub scanned_files: u64,
    pub scanned_bytes: u64,
    pub wasted_bytes: u64,
    pub duration_ms: u64,
    pub cancelled: bool,
}

fn default_min_size() -> u64 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeOptions {
    pub roots: Vec<String>,
    /// `ignore`-style glob patterns (matched against the path relative to each
    /// root, or absolute), e.g. `node_modules`, `*.iso`.
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default = "default_min_size")]
    pub min_size_bytes: u64,
    #[serde(default)]
    pub include_hidden: bool,
    /// Symlinks are never followed by default (PRD §6.3).
    #[serde(default)]
    pub follow_links: bool,
}

/// Per-group delete plan sent by the UI. `keep` must be non-empty — the
/// backend re-hashes `keep[0]` and every `remove` path and refuses any file
/// whose content differs, so identical-content invariant holds even if the
/// scan result is stale.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDeletePlan {
    pub keep: Vec<String>,
    pub remove: Vec<DeleteTarget>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateDeleteRequest {
    pub groups: Vec<GroupDeletePlan>,
    pub mode: DeleteMode,
    #[serde(default)]
    pub dry_run: bool,
}
