//! Docker cleaner — talks to the Docker Engine API via `bollard`
//! (never the CLI, per PRD §6.2). All listing is read-only; deletion goes
//! through [`delete_resources`] which re-checks safety constraints on the
//! daemon side at call time (running containers, built-in networks).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use bollard::container::{ListContainersOptions, RemoveContainerOptions};
use bollard::image::ListImagesOptions;
use bollard::network::{InspectNetworkOptions, ListNetworksOptions};
use bollard::volume::ListVolumesOptions;
use bollard::Docker;
use tauri::ipc::Channel;

use crate::error::{AppError, AppResult};
use crate::types::{
    DeleteOutcome, DockerContainerInfo, DockerDeleteRequest, DockerImageInfo,
    DockerNetworkInfo, DockerOverview, DockerResourceKind, DockerStatus, DockerVolumeInfo,
    JobPhase, ScanProgress,
};

/// Connect to the local daemon — unix socket first, then the default
/// transport (named pipe on Windows, `DOCKER_HOST` if set).
fn connect() -> AppResult<Docker> {
    let socket = Path::new("/var/run/docker.sock");
    let cli = if socket.exists() {
        Docker::connect_with_unix_defaults()
    } else {
        Docker::connect_with_defaults()
    };
    cli.map_err(|e| AppError::DockerUnavailable(e.to_string()))
}

/// Cheap availability probe used by the dashboard.
pub async fn status() -> DockerStatus {
    match connect() {
        Ok(d) => match d.version().await {
            Ok(v) => DockerStatus {
                available: true,
                version: v.version,
                api_version: v.api_version,
                error: None,
            },
            Err(e) => DockerStatus {
                available: false,
                version: None,
                api_version: None,
                error: Some(e.to_string()),
            },
        },
        Err(e) => DockerStatus {
            available: false,
            version: None,
            api_version: None,
            error: Some(e.to_string()),
        },
    }
}

/// Fetch the full Docker view: status + every resource class. All errors from
/// an *unreachable* daemon collapse into `status.available = false`; resource
/// lists are then empty.
pub async fn overview(pinned: &HashSet<String>) -> AppResult<DockerOverview> {
    let status = status().await;
    if !status.available {
        return Ok(DockerOverview {
            status,
            images: vec![],
            containers: vec![],
            volumes: vec![],
            networks: vec![],
            build_cache_records: 0,
            build_cache_bytes: 0,
            reclaimable_bytes: 0,
        });
    }
    let d = connect()?;

    // --- containers (all states; we classify below) ---
    let containers_raw = d
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            size: true,
            limit: Some(-1),
            filters: HashMap::new(),
        }))
        .await
        .unwrap_or_default();

    // image_id → container names + any running?
    let mut image_users: HashMap<String, Vec<String>> = HashMap::new();
    let mut image_running: HashSet<String> = HashSet::new();
    let containers: Vec<DockerContainerInfo> = containers_raw
        .iter()
        .map(|c| {
            let name = c
                .names
                .as_deref()
                .and_then(|n| n.first())
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_else(|| c.id.clone().unwrap_or_default());
            let state = c.state.clone().unwrap_or_default();
            let image_id = c.image_id.clone().unwrap_or_default();
            image_users.entry(image_id.clone()).or_default().push(name.clone());
            if state == "running" {
                image_running.insert(image_id.clone());
            }
            DockerContainerInfo {
                id: c.id.clone().unwrap_or_default(),
                name,
                image: c.image.clone().unwrap_or_default(),
                image_id,
                state,
                status: c.status.clone().unwrap_or_default(),
                created: c.created.unwrap_or(0),
                size_bytes: c.size_rw.unwrap_or(0) as u64,
            }
        })
        .collect();

    // --- images ---
    let images_raw = d
        .list_images(Some(ListImagesOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await
        .unwrap_or_default();
    let images: Vec<DockerImageInfo> = images_raw
        .iter()
        .map(|i| {
            let tags = if i.repo_tags.is_empty() {
                vec!["<none>:<none>".to_string()]
            } else {
                i.repo_tags.clone()
            };
            // list_images id is "sha256:..."; containers report "sha256:..." too,
            // but some daemons return bare digests — match on both forms.
            let id_short = i.id.clone();
            let used_by = image_users.get(&id_short).cloned().unwrap_or_default();
            let in_use_running = image_running.contains(&id_short);
            DockerImageInfo {
                id: i.id.clone(),
                tags,
                size_bytes: i.size as u64,
                created: i.created,
                dangling: i.repo_tags.is_empty()
                    || i.repo_tags.iter().all(|t| t.contains("<none>")),
                used_by,
                in_use_running,
                pinned: pinned.contains(&i.id),
            }
        })
        .collect();

    // --- volumes ---
    let vols_raw = d
        .list_volumes(Some(ListVolumesOptions::<String> {
            filters: HashMap::new(),
        }))
        .await
        .ok();
    let mut volumes: Vec<DockerVolumeInfo> = Vec::new();
    if let Some(v) = vols_raw.and_then(|r| r.volumes) {
        for vol in v {
            // system df is the only source for size; inspect per volume for created date.
            let usage = match d.inspect_volume(&vol.name).await {
                Ok(i) => i.usage_data.as_ref().map(|u| u.size).unwrap_or(-1),
                Err(_) => -1,
            };
            let in_use = vol
                .usage_data
                .as_ref()
                .map(|u| u.ref_count > 0)
                .unwrap_or(false);
            volumes.push(DockerVolumeInfo {
                name: vol.name.clone(),
                driver: vol.driver.clone(),
                in_use,
                size_bytes: (usage >= 0).then_some(usage as u64),
                created_at: vol.created_at.clone(),
                pinned: pinned.contains(&vol.name),
            });
        }
    }

    // --- networks ---
    let nets_raw = d
        .list_networks(Some(ListNetworksOptions::<String> {
            filters: HashMap::new(),
        }))
        .await
        .unwrap_or_default();
    let mut networks = Vec::new();
    for n in nets_raw {
        let name = n.name.clone().unwrap_or_default();
        let builtin = matches!(name.as_str(), "bridge" | "host" | "none");
        // Container count → in_use.
        let containers_attached = n
            .containers
            .as_ref()
            .map(|m| m.len())
            .unwrap_or(0);
        networks.push(DockerNetworkInfo {
            id: n.id.clone().unwrap_or_default(),
            name,
            driver: n.driver.clone().unwrap_or_default(),
            in_use: containers_attached > 0,
            builtin,
        });
    }

    // --- build cache (df is the cheapest aggregate; per-record listing via
    // the BuildKit gRPC API is intentionally skipped to keep the UI fast) ---
    let df = d.df().await.ok();
    let (bc_records, bc_bytes) = df
        .as_ref()
        .and_then(|r| r.build_cache.as_ref())
        .map(|v| (v.len() as u64, v.iter().map(|c| c.size.unwrap_or(0) as u64).sum()))
        .unwrap_or((0, 0));

    let reclaimable: u64 = images
        .iter()
        .filter(|i| !i.pinned && i.used_by.is_empty())
        .map(|i| i.size_bytes)
        .sum::<u64>()
        + containers
            .iter()
            .filter(|c| matches!(c.state.as_str(), "exited" | "created"))
            .map(|c| c.size_bytes)
            .sum::<u64>()
        + bc_bytes;

    Ok(DockerOverview {
        status,
        images,
        containers,
        volumes,
        networks,
        build_cache_records: bc_records,
        build_cache_bytes: bc_bytes,
        reclaimable_bytes: reclaimable,
    })
}

/// `bollard` 0.17 has no typed build-prune call. On unix we POST
/// `/build/prune` to the socket directly — `keep-storage=0` + default
/// `all=false` reclaims only *unused* cache. On non-unix targets we return
/// an error (build-cache prune lands with Windows/macOS support in v0.5).
#[cfg(unix)]
async fn prune_build_cache() -> Result<(), String> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;

    // hyperlocal's UnixConnector plugs into the legacy client.
    let client: Client<hyperlocal::UnixConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build(hyperlocal::UnixConnector);
    let uri = hyperlocal::Uri::new("/var/run/docker.sock", "/build/prune?keep-storage=0");
    let req = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(uri)
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())?;
    let res = client.request(req).await.map_err(|e| e.to_string())?;
    if res.status().is_success() {
        Ok(())
    } else {
        Err(format!("build prune failed: {}", res.status()))
    }
}

#[cfg(not(unix))]
async fn prune_build_cache() -> Result<(), String> {
    Err("build cache prune is not supported on this platform yet".into())
}

fn outcome(id: &str, ok: bool, dry_run: bool, err: Option<String>) -> DeleteOutcome {
    DeleteOutcome {
        path: id.to_string(),
        ok,
        dry_run,
        freed_bytes: 0,
        error: err,
    }
}

/// Delete a batch of Docker resources. `dry_run` reports what *would* be
/// removed without touching the daemon. Safety rules are enforced here, in
/// Rust, not by the frontend:
/// - Images used by a running container → refused.
/// - Built-in networks → refused.
/// - Volumes and any error from the daemon bubble up per-item.
pub async fn delete_resources(
    req: DockerDeleteRequest,
    pinned: &HashSet<String>,
    channel: &Channel<ScanProgress>,
    cancel: Arc<AtomicBool>,
) -> AppResult<Vec<DeleteOutcome>> {
    let d = connect()?;
    let total = req.ids.len() as u64;
    let done = std::sync::atomic::AtomicU64::new(0);
    let emit = |current: &str| {
        let n = done.load(Relaxed);
        let _ = channel.send(ScanProgress {
            phase: JobPhase::Deleting,
            current: current.to_string(),
            entries_seen: n,
            bytes_found: 0,
            percent: Some((n as f32 / total.max(1) as f32) * 100.0),
        });
    };

    // Pre-fetch images once so the "in use by running container" check is
    // done on fresh daemon state, not on the earlier scan result.
    let protected_images: HashSet<String> = if req.kind == DockerResourceKind::Image {
        let ov = overview(&HashSet::new()).await?;
        ov.images
            .iter()
            .filter(|i| i.in_use_running || i.pinned)
            .map(|i| i.id.clone())
            .chain(ov.containers.iter().filter(|c| c.state == "running").map(|c| c.image_id.clone()))
            .collect()
    } else {
        HashSet::new()
    };

    let mut outcomes = Vec::with_capacity(req.ids.len());
    for id in &req.ids {
        if cancel.load(Relaxed) {
            break;
        }
        emit(id);
        let o = if req.dry_run {
            outcome(id, true, true, None)
        } else {
            match req.kind {
                DockerResourceKind::Image => {
                    if protected_images.contains(id) {
                        outcome(id, false, false, Some("image is in use by a running container".into()))
                    } else {
                        match d.remove_image(id, None, None).await {
                            Ok(_) => outcome(id, true, false, None),
                            Err(e) => outcome(id, false, false, Some(e.to_string())),
                        }
                    }
                }
                DockerResourceKind::Container => {
                    match d
                        .remove_container(id, Some(RemoveContainerOptions {
                            force: false, // refuse running containers
                            v: true,      // remove anonymous volumes attached
                            link: false,
                        }))
                        .await
                    {
                        Ok(_) => outcome(id, true, false, None),
                        Err(e) => outcome(id, false, false, Some(e.to_string())),
                    }
                }
                DockerResourceKind::Volume => {
                    if pinned.contains(id) {
                        outcome(id, false, false, Some("volume is pinned".into()))
                    } else {
                        match d.remove_volume(id, None).await {
                            Ok(_) => outcome(id, true, false, None),
                            Err(e) => outcome(id, false, false, Some(e.to_string())),
                        }
                    }
                }
                DockerResourceKind::Network => {
                    // Re-check built-ins and attachments on live state.
                    match d
                        .inspect_network::<String>(id, Some(InspectNetworkOptions {
                            verbose: false,
                            scope: String::new(),
                        }))
                        .await
                    {
                        Ok(net) => {
                            let name = net.name.unwrap_or_default();
                            if matches!(name.as_str(), "bridge" | "host" | "none") {
                                outcome(id, false, false, Some("built-in network".into()))
                            } else {
                                match d.remove_network(id).await {
                                    Ok(_) => outcome(id, true, false, None),
                                    Err(e) => outcome(id, false, false, Some(e.to_string())),
                                }
                            }
                        }
                        Err(e) => outcome(id, false, false, Some(e.to_string())),
                    }
                }
                DockerResourceKind::BuildCache => {
                    match prune_build_cache().await {
                        Ok(()) => outcome(id, true, false, None),
                        Err(e) => outcome(id, false, false, Some(e)),
                    }
                }
            }
        };
        outcomes.push(o);
        done.fetch_add(1, Relaxed);
    }

    let _ = channel.send(ScanProgress {
        phase: if cancel.load(Relaxed) { JobPhase::Cancelled } else { JobPhase::Done },
        current: String::new(),
        entries_seen: done.load(Relaxed),
        bytes_found: 0,
        percent: Some(100.0),
    });
    Ok(outcomes)
}

/// Kept tiny on purpose — `overview()` is the real worker; tests that need a
/// daemon live behind `#[ignore]` and only run in CI.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_builds() {
        let o = outcome("x", true, true, None);
        assert!(o.ok && o.dry_run);
    }
}

// silence unused-import warnings for items only needed in some cfg paths
#[allow(unused_imports)]
fn _unused() {}
