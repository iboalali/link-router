use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Always,
    Ask,
    Browser,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_glob: Option<String>,
    pub action: RuleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomTarget {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default = "default_fallback_target")]
    pub fallback_target: String,
    #[serde(default = "default_true")]
    pub unwrap_redirects: bool,
    #[serde(default)]
    pub strip_tracking_params: bool,
    #[serde(default)]
    pub detect_self_loop: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bypass_query_params: Vec<String>,
    #[serde(default)]
    pub enabled_handlers: Vec<String>,
    #[serde(default)]
    pub targets: Vec<CustomTarget>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainPattern {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_glob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandlerManifest {
    pub id: String,
    pub name: String,
    pub target: String,
    #[serde(default)]
    pub domains: Vec<DomainPattern>,
}

fn default_true() -> bool {
    true
}

fn default_fallback_target() -> String {
    "desktop:google-chrome.desktop".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fallback_target: default_fallback_target(),
            unwrap_redirects: true,
            strip_tracking_params: false,
            detect_self_loop: false,
            bypass_query_params: Vec::new(),
            enabled_handlers: Vec::new(),
            targets: Vec::new(),
            rules: Vec::new(),
        }
    }
}

impl Config {
    pub fn upsert_always_rule(
        &mut self,
        host: &str,
        path_prefix: Option<&str>,
        target_id: &str,
    ) {
        let host_norm = host.trim().to_ascii_lowercase();
        let prefix_norm = path_prefix
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty() && p != "/");

        if let Some(existing) = self.rules.iter_mut().find(|r| {
            r.host.eq_ignore_ascii_case(&host_norm)
                && r.path_prefix == prefix_norm
                && r.path_glob.is_none()
        }) {
            existing.enabled = true;
            existing.action = RuleAction::Always;
            existing.target = Some(target_id.to_string());
            existing.candidates.clear();
            return;
        }

        let slug_base = match &prefix_norm {
            Some(p) => format!("{}{}", host_norm, p),
            None => host_norm.clone(),
        };
        let id = slugify(&slug_base);
        let name = match &prefix_norm {
            Some(p) => format!("{}{}", host_norm, p),
            None => host_norm.clone(),
        };

        self.rules.push(Rule {
            id,
            name,
            enabled: true,
            host: host_norm,
            path_prefix: prefix_norm,
            path_glob: None,
            action: RuleAction::Always,
            target: Some(target_id.to_string()),
            candidates: Vec::new(),
        });
    }

    pub fn set_handler_enabled(&mut self, handler_id: &str, enabled: bool) {
        if enabled {
            if !self.enabled_handlers.iter().any(|h| h == handler_id) {
                self.enabled_handlers.push(handler_id.to_string());
            }
        } else {
            self.enabled_handlers.retain(|h| h != handler_id);
        }
    }

    pub fn is_handler_enabled(&self, handler_id: &str) -> bool {
        self.enabled_handlers.iter().any(|h| h == handler_id)
    }
}

pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "rule".to_string()
    } else {
        out
    }
}

pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return PathBuf::from(xdg).join("link-router");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("link-router")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn handlers_dir() -> PathBuf {
    config_dir().join("handlers.d")
}

pub fn load_config() -> Result<Config, String> {
    load_config_from(&config_path())
}

pub fn load_config_from(path: &Path) -> Result<Config, String> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    toml::from_str(&raw).map_err(|e| format!("failed to parse {}: {}", path.display(), e))
}

pub fn save_config(config: &Config) -> Result<(), String> {
    save_config_to(config, &config_path())
}

pub fn save_config_to(config: &Config, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }
    let rendered =
        toml::to_string_pretty(config).map_err(|e| format!("failed to serialize config: {}", e))?;
    let tmp_path = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    fs::write(&tmp_path, rendered)
        .map_err(|e| format!("failed to write {}: {}", tmp_path.display(), e))?;
    fs::rename(&tmp_path, path)
        .map_err(|e| format!("failed to replace {}: {}", path.display(), e))?;
    Ok(())
}

pub fn load_handler_manifests() -> Vec<HandlerManifest> {
    load_handler_manifests_from(&handlers_dir())
}

pub fn load_handler_manifests_from(dir: &Path) -> Vec<HandlerManifest> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("toml"))
        .collect();
    paths.sort();

    let mut manifests = Vec::new();
    for path in paths {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(manifest) = toml::from_str::<HandlerManifest>(&raw) {
                manifests.push(manifest);
            }
        }
    }
    manifests
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_config_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut cfg = Config::default();
        cfg.fallback_target = "chrome:Default".to_string();
        cfg.upsert_always_rule("*.atlassian.net", Some("/browse/"), "chrome:Profile 1");
        cfg.set_handler_enabled("tempoui", true);

        save_config_to(&cfg, &path).unwrap();
        let loaded = load_config_from(&path).unwrap();
        assert_eq!(cfg, loaded);
    }

    #[test]
    fn upsert_updates_existing_rule_in_place() {
        let mut cfg = Config::default();
        cfg.upsert_always_rule("jira.example.com", None, "chrome:Profile 1");
        cfg.upsert_always_rule("JIRA.EXAMPLE.COM", None, "chrome:Profile 2");
        assert_eq!(cfg.rules.len(), 1);
        assert_eq!(cfg.rules[0].target.as_deref(), Some("chrome:Profile 2"));
    }

    #[test]
    fn loads_handler_manifests_sorted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("b-jira.toml"),
            r#"
id = "work-jira"
name = "Work Jira"
target = "chrome:Profile 1"

[[domains]]
host = "*.atlassian.net"
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("a-docs.toml"),
            r#"
id = "docs"
name = "Docs"
target = "firefox:default"

[[domains]]
host = "docs.rs"
"#,
        )
        .unwrap();

        let manifests = load_handler_manifests_from(dir.path());
        assert_eq!(manifests.len(), 2);
        assert_eq!(manifests[0].id, "docs");
        assert_eq!(manifests[1].id, "work-jira");
    }
}
