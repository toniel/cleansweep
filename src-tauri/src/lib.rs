//! CleanSweep application library.
//!
//! Layout (PRD §8):
//!   rules   — declarative junk-category rules (TOML)
//!   scanner — parallel junk scanner with progress + cancel
//!   dedupe  — size → partial-hash → BLAKE3 duplicate pipeline
//!   docker  — Docker Engine API via bollard (never the CLI)
//!   fsops   — the ONLY module allowed to remove files; path validation,
//!             forbidden list, trash-first, dry-run, idempotent cancel
//!   store   — settings / history / docker pins (tauri-plugin-store JSON)
//!   overview— disk mounts for the dashboard
//!
//! All Tauri commands are thin: they validate input, spawn work on the
//! async runtime or a blocking thread (scan/delete never block the UI),
//! and stream progress back over a `Channel`.

mod dedupe;
mod docker;
mod error;
mod fsops;
mod overview;
mod rules;
mod scanner;
mod store;
mod types;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use error::{AppError, AppResult};
use parking_lot::Mutex;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use types::*;
use uuid::Uuid;

/// One cancel flag per running job, keyed by job id.
/// The UI holds at most one scan + one delete at a time, but job ids make
/// the API race-proof and future-proof.
#[derive(Default)]
struct Jobs {
    cancel: Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>,
}

impl Jobs {
    fn register(&self, job_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancel
            .lock()
            .insert(job_id.to_string(), flag.clone());
        flag
    }

    fn cancel(&self, job_id: &str) -> bool {
        if let Some(f) = self.cancel.lock().get(job_id) {
            f.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn finish(&self, job_id: &str) {
        self.cancel.lock().remove(job_id);
    }
}

fn rules_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("rules"))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn record_history(app: &AppHandle, category: &str, freed: u64, items: u64, mode: &str) {
    let _ = store::add_history(
        app,
        HistoryEntry {
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339(),
            category: category.to_string(),
            freed_bytes: freed,
            item_count: items,
            mode: mode.to_string(),
        },
    );
}

// ---------------------------------------------------------------------------
// Junk scanner
// ---------------------------------------------------------------------------

#[tauri::command]
fn list_junk_categories(app: AppHandle) -> AppResult<Vec<rules::CategoryInfo>> {
    let (cats, _) = rules::load_rules(rules_dir(&app).as_deref())?;
    Ok(cats.iter().map(rules::CategoryInfo::from).collect())
}

#[tauri::command]
async fn scan_junk(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job_id: String,
    categories: Option<Vec<String>>,
    channel: Channel<ScanProgress>,
) -> AppResult<JunkScanResult> {
    let cancel = jobs.register(&job_id);
    let dir = rules_dir(&app);
    let result = tauri::async_runtime::spawn_blocking(move || {
        scanner::run_junk_scan(categories.as_deref(), dir.as_deref(), &channel, cancel)
    })
    .await
    .map_err(|e| AppError::Message(e.to_string()))?;
    jobs.finish(&job_id);
    Ok(result)
}

// ---------------------------------------------------------------------------
// Safe deletion
// ---------------------------------------------------------------------------

#[tauri::command]
async fn delete_items(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job_id: String,
    targets: Vec<DeleteTarget>,
    mode: DeleteMode,
    dry_run: bool,
    channel: Channel<ScanProgress>,
) -> AppResult<DeleteResult> {
    if targets.is_empty() {
        return Err(AppError::Message("nothing to delete".into()));
    }
    let cancel = jobs.register(&job_id);
    let cancel2 = cancel.clone();
    let total = targets.len() as u64;
    let done = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let done2 = done.clone();

    let outcomes = tauri::async_runtime::spawn_blocking(move || {
        fsops::delete_paths(&targets, mode, dry_run, &cancel2, |o| {
            let n = done2.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = channel.send(ScanProgress {
                phase: JobPhase::Deleting,
                current: o.path.clone(),
                entries_seen: n,
                bytes_found: 0,
                percent: Some(n as f32 / total.max(1) as f32 * 100.0),
            });
        })
    })
    .await
    .map_err(|e| AppError::Message(e.to_string()))?;
    jobs.finish(&job_id);

    let freed: u64 = outcomes.iter().filter(|o| o.ok).map(|o| o.freed_bytes).sum();
    if !dry_run && freed > 0 {
        record_history(
            &app,
            "junk",
            freed,
            outcomes.iter().filter(|o| o.ok).count() as u64,
            match mode {
                DeleteMode::Trash => "trash",
                DeleteMode::Permanent => "permanent",
            },
        );
    }
    Ok(DeleteResult {
        cancelled: cancel.load(Ordering::Relaxed),
        freed_bytes: freed,
        outcomes,
    })
}

// ---------------------------------------------------------------------------
// Docker
// ---------------------------------------------------------------------------

fn pins_set(app: &AppHandle) -> HashSet<String> {
    store::list_pins(app).unwrap_or_default().into_iter().collect()
}

#[tauri::command]
async fn docker_status() -> DockerStatus {
    docker::status().await
}

#[tauri::command]
async fn docker_overview(app: AppHandle) -> AppResult<DockerOverview> {
    let pinned = pins_set(&app);
    docker::overview(&pinned).await
}

#[tauri::command]
async fn docker_delete(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job_id: String,
    req: DockerDeleteRequest,
    channel: Channel<ScanProgress>,
) -> AppResult<Vec<DeleteOutcome>> {
    let cancel = jobs.register(&job_id);
    let pinned = pins_set(&app);
    let res = docker::delete_resources(req, &pinned, &channel, cancel).await;
    jobs.finish(&job_id);
    let outcomes = res?;
    if !outcomes.iter().all(|o| o.dry_run) {
        let ok = outcomes.iter().filter(|o| o.ok).count() as u64;
        if ok > 0 {
            record_history(&app, "docker", 0, ok, "docker");
        }
    }
    Ok(outcomes)
}

#[tauri::command]
fn docker_set_pin(app: AppHandle, id: String, pinned: bool) -> AppResult<Vec<String>> {
    store::set_pin(&app, &id, pinned)
}

#[tauri::command]
fn docker_list_pins(app: AppHandle) -> AppResult<Vec<String>> {
    store::list_pins(&app)
}

// ---------------------------------------------------------------------------
// Duplicates
// ---------------------------------------------------------------------------

#[tauri::command]
async fn scan_duplicates(
    jobs: State<'_, Jobs>,
    job_id: String,
    opts: DedupeOptions,
    channel: Channel<ScanProgress>,
) -> AppResult<DuplicateScanResult> {
    let cancel = jobs.register(&job_id);
    let res = tauri::async_runtime::spawn_blocking(move || dedupe::run(opts, &channel, cancel))
        .await
        .map_err(|e| AppError::Message(e.to_string()))?;
    jobs.finish(&job_id);
    res
}

/// Delete duplicates per-group. For every group the backend re-verifies
/// that `keep[0]` and each `remove` path still have identical content —
/// so a stale scan can never delete a changed file.
#[tauri::command]
async fn delete_duplicates(
    app: AppHandle,
    jobs: State<'_, Jobs>,
    job_id: String,
    req: DuplicateDeleteRequest,
    channel: Channel<ScanProgress>,
) -> AppResult<DeleteResult> {
    // Flatten the plan into one target list, re-verifying content first.
    let mut targets: Vec<DeleteTarget> = Vec::new();
    for g in &req.groups {
        let Some(keep) = g.keep.first() else {
            return Err(AppError::Message("each group must keep at least one file".into()));
        };
        let keep_path = PathBuf::from(keep);
        for r in &g.remove {
            if r.path == *keep {
                return Err(AppError::Message("keep and remove path overlap".into()));
            }
            let rp = Path::new(&r.path);
            if !rp.exists() {
                continue; // already gone — treat as ok, idempotent
            }
            if !dedupe::verify_identical(&keep_path, rp) {
                return Err(AppError::Message(format!(
                    "refused: {} no longer matches {}",
                    r.path, keep
                )));
            }
            targets.push(r.clone());
        }
    }

    let cancel = jobs.register(&job_id);
    let cancel2 = cancel.clone();
    let mode = req.mode;
    let dry_run = req.dry_run;
    let total = targets.len() as u64;
    let done = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let done2 = done.clone();
    let outcomes = tauri::async_runtime::spawn_blocking(move || {
        fsops::delete_paths(&targets, mode, dry_run, &cancel2, |o| {
            let n = done2.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = channel.send(ScanProgress {
                phase: JobPhase::Deleting,
                current: o.path.clone(),
                entries_seen: n,
                bytes_found: 0,
                percent: Some(n as f32 / total.max(1) as f32 * 100.0),
            });
        })
    })
    .await
    .map_err(|e| AppError::Message(e.to_string()))?;
    jobs.finish(&job_id);

    let freed: u64 = outcomes.iter().filter(|o| o.ok).map(|o| o.freed_bytes).sum();
    if !dry_run && freed > 0 {
        record_history(
            &app,
            "duplicates",
            freed,
            outcomes.iter().filter(|o| o.ok).count() as u64,
            match mode {
                DeleteMode::Trash => "trash",
                DeleteMode::Permanent => "permanent",
            },
        );
    }
    Ok(DeleteResult {
        outcomes,
        freed_bytes: freed,
        cancelled: cancel.load(Ordering::Relaxed),
    })
}

// ---------------------------------------------------------------------------
// Misc commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn cancel_job(jobs: State<'_, Jobs>, job_id: String) -> bool {
    jobs.cancel(&job_id)
}

#[tauri::command]
fn list_drives() -> Vec<DriveInfo> {
    overview::list_drives()
}

#[tauri::command]
fn get_settings(app: AppHandle) -> AppResult<store::Settings> {
    store::get_settings(&app)
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: store::Settings) -> AppResult<()> {
    store::save_settings(&app, &settings)
}

#[tauri::command]
fn list_history(app: AppHandle) -> AppResult<Vec<HistoryEntry>> {
    store::list_history(&app)
}

#[tauri::command]
fn clear_history(app: AppHandle) -> AppResult<()> {
    store::clear_history(&app)
}

/// Used by the duplicate-finder "open location" action — reveals the file
/// in the OS file manager through the opener plugin on the frontend.
#[tauri::command]
fn reveal_path(path: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(AppError::Message("path does not exist".into()));
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Jobs::default())
        .invoke_handler(tauri::generate_handler![
            list_junk_categories,
            scan_junk,
            delete_items,
            docker_status,
            docker_overview,
            docker_delete,
            docker_set_pin,
            docker_list_pins,
            scan_duplicates,
            delete_duplicates,
            cancel_job,
            list_drives,
            get_settings,
            save_settings,
            list_history,
            clear_history,
            reveal_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running CleanSweep");
}
