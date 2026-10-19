//! Local persistence: settings, cleanup history, and Docker pin lists.
//! Backed by `tauri-plugin-store` (flat JSON in the app config dir) —
//! deliberately simple; there is no relational data worth SQLite.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Wry};
use tauri_plugin_store::StoreExt;

use crate::error::{AppError, AppResult};
use crate::types::HistoryEntry;

const STORE_FILE: &str = "cleansweep.json";
const HISTORY_KEY: &str = "history";
const PINS_KEY: &str = "docker_pins";
const SETTINGS_KEY: &str = "settings";
/// Cap history so the JSON file stays small over years of use.
const HISTORY_MAX: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// "id" | "en"
    pub language: String,
    /// "light" | "dark" | "system"
    pub theme: String,
    /// Default delete action: "trash" | "permanent".
    pub default_delete_mode: String,
    /// Global exclude patterns for duplicate scans.
    pub excludes: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "id".into(),
            theme: "system".into(),
            default_delete_mode: "trash".into(),
            excludes: vec![
                "node_modules".into(),
                ".git".into(),
                "target".into(),
                "*.iso".into(),
            ],
        }
    }
}

fn store(app: &AppHandle<Wry>) -> AppResult<std::sync::Arc<tauri_plugin_store::Store<Wry>>> {
    app.store(STORE_FILE).map_err(|e| AppError::Message(e.to_string()))
}

pub fn get_settings(app: &AppHandle<Wry>) -> AppResult<Settings> {
    let s = store(app)?;
    Ok(s
        .get(SETTINGS_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn save_settings(app: &AppHandle<Wry>, settings: &Settings) -> AppResult<()> {
    let s = store(app)?;
    s.set(
        SETTINGS_KEY,
        serde_json::to_value(settings).map_err(|e| AppError::Message(e.to_string()))?,
    );
    s.save().map_err(|e| AppError::Message(e.to_string()))
}

pub fn list_history(app: &AppHandle<Wry>) -> AppResult<Vec<HistoryEntry>> {
    let s = store(app)?;
    Ok(s
        .get(HISTORY_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn add_history(app: &AppHandle<Wry>, entry: HistoryEntry) -> AppResult<()> {
    let s = store(app)?;
    let mut items: Vec<HistoryEntry> = s
        .get(HISTORY_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    items.insert(0, entry);
    items.truncate(HISTORY_MAX);
    s.set(HISTORY_KEY, serde_json::to_value(items).map_err(|e| AppError::Message(e.to_string()))?);
    s.save().map_err(|e| AppError::Message(e.to_string()))
}

pub fn clear_history(app: &AppHandle<Wry>) -> AppResult<()> {
    let s = store(app)?;
    s.set(HISTORY_KEY, serde_json::json!([]));
    s.save().map_err(|e| AppError::Message(e.to_string()))
}

/// Docker pin list: image ids and volume names the user marked as
/// never-delete (PRD §6.2 whitelist).
pub fn list_pins(app: &AppHandle<Wry>) -> AppResult<Vec<String>> {
    let s = store(app)?;
    Ok(s
        .get(PINS_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn set_pin(app: &AppHandle<Wry>, id: &str, pinned: bool) -> AppResult<Vec<String>> {
    let s = store(app)?;
    let mut pins: Vec<String> = s
        .get(PINS_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if pinned && !pins.iter().any(|p| p == id) {
        pins.push(id.to_string());
    } else if !pinned {
        pins.retain(|p| p != id);
    }
    s.set(PINS_KEY, serde_json::to_value(&pins).map_err(|e| AppError::Message(e.to_string()))?);
    s.save().map_err(|e| AppError::Message(e.to_string()))?;
    Ok(pins)
}
