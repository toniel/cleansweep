//! Declarative junk-category rules.
//!
//! Rules are plain TOML files so new categories can be added without touching
//! core code (PRD §6.1). A base rule set is embedded in the binary; users can
//! drop extra `.toml` files in `<config>/cleansweep/rules/` and they are merged.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::types::RiskLevel;

/// How a candidate path in a rule is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// Literal directory path (after variable expansion).
    #[default]
    Dir,
    /// Match *child entries* of the expanded base dir against `pattern`
    /// (filesystem glob: `*`, `?`, `[..]`).
    GlobChildren,
    /// Find directories with this marker name during traversal of `dir`
    /// (e.g. `__pycache__`).
    MarkerDir,
}

/// One cleanable target inside a category.
#[derive(Debug, Clone, Deserialize)]
pub struct TargetRule {
    /// Path with `$VAR` placeholders, e.g. `$HOME_CACHE/foo`.
    /// For `glob_children` this is the *base* directory to search in;
    /// for `marker_dir` the root to traverse.
    pub dir: String,
    #[serde(default)]
    pub kind: TargetKind,
    /// Pattern for `glob_children`, marker name for `marker_dir`.
    pub pattern: Option<String>,
    /// How deep `marker_dir` traversal may descend from `dir` (default 6).
    pub max_depth: Option<usize>,
    /// Platform filter: "linux", "windows", "macos". Empty = all.
    #[serde(default)]
    pub platforms: Vec<String>,
}

/// A junk category loaded from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct CategoryRule {
    pub id: String,
    /// i18n key, e.g. "cat.system.temp" — frontend falls back to `label`.
    #[serde(default)]
    pub label_key: Option<String>,
    pub label: String,
    #[serde(default)]
    pub desc_key: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_risk", alias = "risk")]
    pub risk: RiskKind,
    #[serde(default)]
    pub targets: Vec<TargetRule>,
}

fn default_risk() -> RiskKind {
    RiskKind::Safe
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RiskKind {
    #[default]
    Safe,
    Caution,
}

impl From<RiskKind> for RiskLevel {
    fn from(r: RiskKind) -> Self {
        match r {
            RiskKind::Safe => RiskLevel::Safe,
            RiskKind::Caution => RiskLevel::Caution,
        }
    }
}

/// Top-level TOML document: `[[category]]` array + optional `[scan]` tuning.
#[derive(Debug, Deserialize)]
struct RulesFile {
    category: Vec<CategoryRule>,
    #[serde(default)]
    scan: ScanTuning,
}

/// Tunable scanner options from the TOML `[scan]` section.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ScanTuning {
    /// Directory names skipped during `marker_dir` traversal.
    #[serde(default)]
    pub marker_dir_exclude: Vec<String>,
}

/// Category metadata returned to the frontend before scanning.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
    pub id: String,
    pub label: String,
    pub label_key: Option<String>,
    pub description: String,
    pub desc_key: Option<String>,
    pub risk: crate::types::RiskLevel,
}

impl From<&CategoryRule> for CategoryInfo {
    fn from(r: &CategoryRule) -> Self {
        Self {
            id: r.id.clone(),
            label: r.label.clone(),
            label_key: r.label_key.clone(),
            description: r.description.clone(),
            desc_key: r.desc_key.clone(),
            risk: r.risk.into(),
        }
    }
}

fn cargo_home() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("CARGO_HOME") {
        return Some(PathBuf::from(v));
    }
    home::home_dir().map(|h| h.join(".cargo"))
}

fn go_path() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("GOPATH") {
        return Some(PathBuf::from(v));
    }
    home::home_dir().map(|h| h.join("go"))
}

/// Environment-variable → directory map used for `$VAR` expansion.
fn var_map() -> HashMap<&'static str, Option<PathBuf>> {
    let home = home::home_dir();
    let mut m: HashMap<&'static str, Option<PathBuf>> = HashMap::new();
    m.insert("HOME", home.clone());
    m.insert("HOME_CACHE", home.as_ref().map(|h| h.join(".cache")));
    m.insert("HOME_CONFIG", home.as_ref().map(|h| h.join(".config")));
    m.insert("HOME_LOCAL_SHARE", home.as_ref().map(|h| h.join(".local/share")));
    m.insert("TMPDIR", Some(std::env::temp_dir()));
    m.insert("CARGO_HOME", cargo_home());
    m.insert("GOPATH", go_path());
    m
}

/// Expand a leading `$VAR` in a rule path. Returns None if the variable is
/// unknown or unset on this machine (target is simply skipped).
pub fn expand_path(raw: &str) -> Option<PathBuf> {
    lazy_static::lazy_static! {
        static ref VARS: HashMap<&'static str, Option<PathBuf>> = var_map();
    }

    if let Some(stripped) = raw.strip_prefix('$') {
        let end = stripped.find(['/', '\\']).unwrap_or(stripped.len());
        let (name, tail) = stripped.split_at(end);
        let base = VARS.get(name)?.clone()?;
        let tail = tail.trim_start_matches(['/', '\\']);
        return Some(if tail.is_empty() { base } else { base.join(tail) });
    }
    if let Some(tail) = raw.strip_prefix('~') {
        let base = home::home_dir()?;
        let tail = tail.trim_start_matches(['/', '\\']);
        return Some(if tail.is_empty() { base } else { base.join(tail) });
    }
    Some(PathBuf::from(raw))
}

fn current_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Parse a single TOML document into category rules.
fn parse_rules_file(toml_str: &str) -> AppResult<RulesFile> {
    Ok(toml::from_str(toml_str)?)
}

/// Load embedded base rules + user rules from `extra_dir`.
/// Duplicate `id`s in user rules override base categories wholesale.
/// Returns `(categories, tuning)`.
pub fn load_rules(extra_dir: Option<&Path>) -> AppResult<(Vec<CategoryRule>, ScanTuning)> {
    let base = parse_rules_file(BASE_RULES_TOML)?;
    let mut rules = base.category;
    let mut tuning = base.scan;

    if let Some(dir) = extra_dir {
        if dir.is_dir() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
                .map_err(AppError::Io)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map(|e| e == "toml").unwrap_or(false))
                .collect();
            files.sort();
            for f in files {
                let text = std::fs::read_to_string(&f).map_err(AppError::Io)?;
                let extra = parse_rules_file(&text)
                    .map_err(|e| AppError::Message(format!("{}: {}", f.display(), e)))?;
                if !extra.scan.marker_dir_exclude.is_empty() {
                    tuning = extra.scan;
                }
                for cat in extra.category {
                    if let Some(existing) = rules.iter_mut().find(|r| r.id == cat.id) {
                        *existing = cat;
                    } else {
                        rules.push(cat);
                    }
                }
            }
        }
    }

    // Drop categories that end up with no targets for this platform.
    rules.retain(|c| c.targets.iter().any(target_applies));
    Ok((rules, tuning))
}

/// Whether a target applies to the current platform.
pub fn target_applies(t: &TargetRule) -> bool {
    t.platforms.is_empty() || t.platforms.iter().any(|p| p == current_platform())
}

/// Built-in base rules — Linux-first per PRD platform priority.
const BASE_RULES_TOML: &str = include_str!("../rules/junk_rules.toml");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_embedded_rules() {
        let (rules, tuning) = load_rules(None).unwrap();
        assert!(
            rules.len() >= 5,
            "expected >=5 categories, got {}",
            rules.len()
        );
        assert!(rules.iter().all(|r| !r.id.is_empty() && !r.targets.is_empty()));
        assert!(tuning.marker_dir_exclude.contains(&"node_modules".to_string()));
    }

    #[test]
    fn expands_home_vars() {
        let p = expand_path("$HOME_CACHE/foo").unwrap();
        assert!(p.ends_with(".cache/foo"));
        assert!(p.is_absolute());
    }

    #[test]
    fn unknown_var_is_none() {
        assert!(expand_path("$NOPE_123/x").is_none());
    }

    #[test]
    fn user_rule_overrides_base() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("override.toml"),
            r#"[[category]]
id = "system.temp"
label = "Overridden"
targets = [{ dir = "$HOME_CACHE/nonexistent" }]
"#,
        )
        .unwrap();
        let (rules, _) = load_rules(Some(tmp.path())).unwrap();
        let cat = rules.iter().find(|r| r.id == "system.temp").unwrap();
        assert_eq!(cat.label, "Overridden");
    }
}
