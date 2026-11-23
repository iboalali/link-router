use crate::config::Config;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    ChromeProfile,
    ChromeIncognito,
    FirefoxProfile,
    DesktopApp,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetInfo {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub icon_name: String,
    pub kind: TargetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_dir: Option<String>,
}

pub fn discover_targets(config: &Config) -> Vec<TargetInfo> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();

    for t in discover_chrome_profiles() {
        if seen.insert(t.id.clone()) {
            targets.push(t);
        }
    }

    for t in discover_firefox_profiles() {
        if seen.insert(t.id.clone()) {
            targets.push(t);
        }
    }

    for t in discover_desktop_apps() {
        if seen.insert(t.id.clone()) {
            targets.push(t);
        }
    }

    for custom in &config.targets {
        let id = format!("custom:{}", custom.id);
        if seen.insert(id.clone()) {
            targets.push(TargetInfo {
                id,
                name: custom.name.clone(),
                subtitle: custom.command.clone(),
                icon_name: "utilities-terminal".to_string(),
                kind: TargetKind::Custom,
                email: None,
                profile_dir: None,
            });
        }
    }

    targets
}

pub fn discover_chrome_profiles() -> Vec<TargetInfo> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let local_state = PathBuf::from(&home)
        .join(".config")
        .join("google-chrome")
        .join("Local State");
    let mut profiles = discover_chrome_profiles_from(&local_state);

    if profiles.is_empty() && find_in_path("google-chrome").is_some() {
        profiles.push(TargetInfo {
            id: "chrome:Default".to_string(),
            name: "Google Chrome (Default)".to_string(),
            subtitle: "Profile: Default".to_string(),
            icon_name: "google-chrome".to_string(),
            kind: TargetKind::ChromeProfile,
            email: None,
            profile_dir: Some("Default".to_string()),
        });
        profiles.push(TargetInfo {
            id: "chrome-incognito:Default".to_string(),
            name: "Google Chrome (Incognito)".to_string(),
            subtitle: "Incognito window".to_string(),
            icon_name: "google-chrome".to_string(),
            kind: TargetKind::ChromeIncognito,
            email: None,
            profile_dir: Some("Default".to_string()),
        });
    }

    profiles
}

pub fn discover_chrome_profiles_from(local_state_path: &Path) -> Vec<TargetInfo> {
    let Ok(raw) = fs::read_to_string(local_state_path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    let Some(cache) = json
        .get("profile")
        .and_then(|p| p.get("info_cache"))
        .and_then(|c| c.as_object())
    else {
        return Vec::new();
    };

    let mut dirs: Vec<(&String, &Value)> = cache.iter().collect();
    dirs.sort_by(|a, b| a.0.cmp(b.0));

    let mut out = Vec::new();
    for (dir, info) in dirs {
        let display_name = info
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(dir);
        let email = info
            .get("user_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let subtitle = match &email {
            Some(e) => format!("{} ({})", e, dir),
            None => format!("Profile: {}", dir),
        };
        out.push(TargetInfo {
            id: format!("chrome:{}", dir),
            name: format!("Chrome ({})", display_name),
            subtitle,
            icon_name: "google-chrome".to_string(),
            kind: TargetKind::ChromeProfile,
            email,
            profile_dir: Some(dir.clone()),
        });
    }

    if let Some(first_dir) = out.first().and_then(|t| t.profile_dir.clone()) {
        out.push(TargetInfo {
            id: format!("chrome-incognito:{}", first_dir),
            name: "Chrome (Incognito)".to_string(),
            subtitle: "Incognito window".to_string(),
            icon_name: "google-chrome".to_string(),
            kind: TargetKind::ChromeIncognito,
            email: None,
            profile_dir: Some(first_dir),
        });
    }

    out
}

pub fn discover_firefox_profiles() -> Vec<TargetInfo> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let candidates = [
        PathBuf::from(&home).join("snap/firefox/common/.mozilla/firefox/profiles.ini"),
        PathBuf::from(&home).join(".mozilla/firefox/profiles.ini"),
    ];
    for ini in candidates {
        let profiles = discover_firefox_profiles_from(&ini);
        if !profiles.is_empty() {
            return profiles;
        }
    }
    Vec::new()
}

pub fn discover_firefox_profiles_from(ini_path: &Path) -> Vec<TargetInfo> {
    let Ok(raw) = fs::read_to_string(ini_path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut in_profile = false;
    let mut name: Option<String> = None;

    let flush = |name: &mut Option<String>, out: &mut Vec<TargetInfo>| {
        if let Some(n) = name.take() {
            out.push(TargetInfo {
                id: format!("firefox:{}", n),
                name: format!("Firefox ({})", n),
                subtitle: format!("Firefox profile: {}", n),
                icon_name: "firefox".to_string(),
                kind: TargetKind::FirefoxProfile,
                email: None,
                profile_dir: Some(n),
            });
        }
    };

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            flush(&mut name, &mut out);
            in_profile = trimmed.starts_with("[Profile");
            continue;
        }
        if in_profile {
            if let Some(val) = trimmed.strip_prefix("Name=") {
                name = Some(val.trim().to_string());
            }
        }
    }
    flush(&mut name, &mut out);
    out
}

pub fn discover_desktop_apps() -> Vec<TargetInfo> {
    let mut dirs = Vec::new();
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.trim().is_empty() {
            dirs.push(PathBuf::from(xdg_data).join("applications"));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/applications"));
        dirs.push(PathBuf::from(&home).join(".local/share/flatpak/exports/share/applications"));
    }
    dirs.push(PathBuf::from("/usr/share/applications"));
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

    let mut seen = HashSet::new();
    let mut apps = Vec::new();

    for dir in dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|entry| entry.path()))
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("desktop"))
            .collect();
        paths.sort();

        for path in paths {
            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if file_name == "link-router.desktop" || !seen.insert(file_name.to_string()) {
                continue;
            }
            if let Some(entry) = parse_desktop_entry(&path) {
                if entry.handles_http {
                    apps.push(TargetInfo {
                        id: format!("desktop:{}", file_name),
                        name: entry.name,
                        subtitle: file_name.to_string(),
                        icon_name: entry.icon.unwrap_or_else(|| "web-browser".to_string()),
                        kind: TargetKind::DesktopApp,
                        email: None,
                        profile_dir: None,
                    });
                }
            }
        }
    }

    apps
}

#[derive(Debug, Clone)]
struct DesktopEntry {
    name: String,
    exec: String,
    icon: Option<String>,
    handles_http: bool,
}

fn parse_desktop_entry(path: &Path) -> Option<DesktopEntry> {
    let raw = fs::read_to_string(path).ok()?;
    let mut in_main = false;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut icon: Option<String> = None;
    let mut mime_types = String::new();
    let mut no_display = false;
    let mut hidden = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_main = trimmed == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("Name=") {
            if name.is_none() {
                name = Some(v.trim().to_string());
            }
        } else if let Some(v) = trimmed.strip_prefix("Exec=") {
            exec = Some(v.trim().to_string());
        } else if let Some(v) = trimmed.strip_prefix("Icon=") {
            icon = Some(v.trim().to_string());
        } else if let Some(v) = trimmed.strip_prefix("MimeType=") {
            mime_types = v.trim().to_string();
        } else if let Some(v) = trimmed.strip_prefix("NoDisplay=") {
            no_display = v.trim().eq_ignore_ascii_case("true");
        } else if let Some(v) = trimmed.strip_prefix("Hidden=") {
            hidden = v.trim().eq_ignore_ascii_case("true");
        }
    }

    if hidden || no_display {
        return None;
    }

    let handles_http = mime_types.split(';').any(|m| {
        let m = m.trim();
        m == "x-scheme-handler/http" || m == "x-scheme-handler/https"
    });

    Some(DesktopEntry {
        name: name?,
        exec: exec?,
        icon,
        handles_http,
    })
}

fn find_desktop_file(desktop_id: &str) -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.trim().is_empty() {
            dirs.push(PathBuf::from(xdg_data).join("applications"));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/applications"));
        dirs.push(PathBuf::from(&home).join(".local/share/flatpak/exports/share/applications"));
    }
    dirs.push(PathBuf::from("/usr/share/applications"));
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

    for dir in dirs {
        let candidate = dir.join(desktop_id);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

pub fn resolve_command(
    target_id: &str,
    url: &str,
    config: &Config,
) -> Result<(String, Vec<String>), String> {
    if let Some(profile_dir) = target_id.strip_prefix("chrome:") {
        let bin = find_chrome_binary().unwrap_or_else(|| "google-chrome".to_string());
        return Ok((
            bin,
            vec![
                format!("--profile-directory={}", profile_dir),
                url.to_string(),
            ],
        ));
    }

    if let Some(profile_dir) = target_id.strip_prefix("chrome-incognito:") {
        let bin = find_chrome_binary().unwrap_or_else(|| "google-chrome".to_string());
        return Ok((
            bin,
            vec![
                "--incognito".to_string(),
                format!("--profile-directory={}", profile_dir),
                url.to_string(),
            ],
        ));
    }

    if let Some(profile_name) = target_id.strip_prefix("firefox:") {
        return Ok((
            "firefox".to_string(),
            vec![
                "-P".to_string(),
                profile_name.to_string(),
                url.to_string(),
            ],
        ));
    }

    if let Some(desktop_id) = target_id.strip_prefix("desktop:") {
        if desktop_id == "link-router.desktop" {
            return Err("refusing to launch link-router.desktop (loop prevention)".to_string());
        }
        let path = find_desktop_file(desktop_id)
            .ok_or_else(|| format!("desktop file not found: {}", desktop_id))?;
        let entry = parse_desktop_entry(&path)
            .ok_or_else(|| format!("invalid desktop file: {}", path.display()))?;
        let parts = split_desktop_exec(&entry.exec, url);
        let (program, args) = parts
            .split_first()
            .ok_or_else(|| format!("empty Exec line in {}", desktop_id))?;
        guard_against_loop(program)?;
        return Ok((program.clone(), args.to_vec()));
    }

    if let Some(custom_id) = target_id.strip_prefix("custom:") {
        let custom = config
            .targets
            .iter()
            .find(|t| t.id == custom_id)
            .ok_or_else(|| format!("custom target not found: {}", custom_id))?;
        guard_against_loop(&custom.command)?;
        let mut args = Vec::new();
        let mut replaced = false;
        for arg in &custom.args {
            if arg.contains("%u") || arg.contains("%U") {
                args.push(arg.replace("%u", url).replace("%U", url));
                replaced = true;
            } else {
                args.push(arg.clone());
            }
        }
        if !replaced {
            args.push(url.to_string());
        }
        return Ok((custom.command.clone(), args));
    }

    Err(format!("unknown target id: {}", target_id))
}

fn guard_against_loop(program: &str) -> Result<(), String> {
    let base = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    if base == "xdg-open" || base == "link-router" {
        return Err(format!(
            "refusing to launch '{}' directly (loop prevention)",
            base
        ));
    }
    Ok(())
}

pub fn launch_target(target_id: &str, url: &str, config: &Config) -> Result<(), String> {
    let (program, args) = resolve_command(target_id, url, config)?;
    guard_against_loop(&program)?;

    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Preserve Wayland activation tokens so GNOME raises the launched target window.
    for key in ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"] {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }

    cmd.spawn()
        .map_err(|e| format!("failed to launch '{}' ({:?}): {}", program, args, e))?;
    Ok(())
}

pub fn split_desktop_exec(exec: &str, url: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';
    let mut chars = exec.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if ch == quote_char {
                in_quotes = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quotes = true;
            quote_char = ch;
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut out = Vec::new();
    let mut inserted_url = false;
    for tok in tokens {
        match tok.as_str() {
            "%u" | "%U" | "%f" | "%F" => {
                out.push(url.to_string());
                inserted_url = true;
            }
            "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
            _ => {
                if tok.contains("%u") || tok.contains("%U") {
                    out.push(tok.replace("%u", url).replace("%U", url));
                    inserted_url = true;
                } else {
                    out.push(tok);
                }
            }
        }
    }
    if !inserted_url && !out.is_empty() {
        out.push(url.to_string());
    }
    out
}

fn find_chrome_binary() -> Option<String> {
    for candidate in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"] {
        if let Some(p) = find_in_path(candidate) {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

fn find_in_path(bin: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let full = dir.join(bin);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

pub fn ancestor_executable_names() -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut pid = read_ppid(std::process::id());
    for _ in 0..32 {
        let Some(current) = pid else {
            break;
        };
        if current <= 1 {
            break;
        }
        if let Ok(exe) = fs::read_link(format!("/proc/{}/exe", current)) {
            if let Some(base) = exe.file_name().and_then(|s| s.to_str()) {
                let cleaned = base.strip_suffix(" (deleted)").unwrap_or(base).to_ascii_lowercase();
                if !cleaned.is_empty() && seen.insert(cleaned.clone()) {
                    names.push(cleaned);
                }
            }
        }
        if let Ok(comm) = fs::read_to_string(format!("/proc/{}/comm", current)) {
            let cleaned = comm.trim().to_ascii_lowercase();
            if !cleaned.is_empty() && seen.insert(cleaned.clone()) {
                names.push(cleaned);
            }
        }
        pid = read_ppid(current);
    }
    names
}

fn read_ppid(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    let rparen = stat.rfind(')')?;
    let rest = stat.get(rparen + 2..)?;
    rest.split_whitespace().nth(1)?.parse::<u32>().ok()
}

pub fn target_matches_ancestors(
    target_id: &str,
    url: &str,
    config: &Config,
    ancestors: &[String],
) -> bool {
    if ancestors.is_empty() || target_id == config.fallback_target {
        return false;
    }
    if !(target_id.starts_with("custom:") || target_id.starts_with("desktop:")) {
        return false;
    }
    let Ok((program, _)) = resolve_command(target_id, url, config) else {
        return false;
    };
    let target_base = Path::new(&program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&program)
        .to_ascii_lowercase();
    if target_base.is_empty() {
        return false;
    }
    ancestors.iter().any(|anc| {
        anc == &target_base
            || (target_base.len() >= 4 && anc.starts_with(&format!("{}-", target_base)))
            || (anc.len() >= 4 && target_base.starts_with(&format!("{}-", anc)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CustomTarget;

    #[test]
    fn parses_chrome_local_state_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("Local State");
        fs::write(
            &state,
            r#"{
              "profile": {
                "info_cache": {
                  "Default": { "name": "Personal", "user_name": "personal@example.com" },
                  "Profile 1": { "name": "Work", "user_name": "work@acme.example" }
                }
              }
            }"#,
        )
        .unwrap();

        let profiles = discover_chrome_profiles_from(&state);
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles[0].id, "chrome:Default");
        assert_eq!(profiles[0].name, "Chrome (Personal)");
        assert_eq!(profiles[1].id, "chrome:Profile 1");
        assert_eq!(profiles[1].name, "Chrome (Work)");
        assert_eq!(profiles[1].email.as_deref(), Some("work@acme.example"));
        assert_eq!(profiles[2].kind, TargetKind::ChromeIncognito);
    }

    #[test]
    fn parses_firefox_profiles_ini() {
        let dir = tempfile::tempdir().unwrap();
        let ini = dir.path().join("profiles.ini");
        fs::write(
            &ini,
            "[General]\nStartWithLastProfile=1\n\n[Profile0]\nName=default\nPath=abc.default\n\n[Profile1]\nName=work\nPath=xyz.work\n",
        )
        .unwrap();

        let profiles = discover_firefox_profiles_from(&ini);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].id, "firefox:default");
        assert_eq!(profiles[1].id, "firefox:work");
    }

    #[test]
    fn splits_desktop_exec_with_quotes_and_field_codes() {
        let args = split_desktop_exec(
            r#"/usr/bin/google-chrome-stable --new-window "%U" %i"#,
            "https://acme.atlassian.net/browse/TM-1",
        );
        assert_eq!(
            args,
            vec![
                "/usr/bin/google-chrome-stable",
                "--new-window",
                "https://acme.atlassian.net/browse/TM-1"
            ]
        );
    }

    #[test]
    fn rejects_loop_targets() {
        let mut cfg = Config::default();
        cfg.targets.push(CustomTarget {
            id: "bad".to_string(),
            name: "Bad".to_string(),
            command: "xdg-open".to_string(),
            args: vec!["%u".to_string()],
        });

        assert!(resolve_command("desktop:link-router.desktop", "https://example.com", &cfg).is_err());
        assert!(resolve_command("custom:bad", "https://example.com", &cfg).is_err());
    }

    #[test]
    fn matches_self_loop_ancestors_for_custom_targets() {
        let mut cfg = Config::default();
        cfg.targets.push(CustomTarget {
            id: "betterjira".to_string(),
            name: "Better Jira".to_string(),
            command: "/home/user/.local/bin/betterjira".to_string(),
            args: vec!["open".to_string(), "%u".to_string()],
        });

        let url = "https://acme.atlassian.net/browse/TM-1";
        assert!(target_matches_ancestors(
            "custom:betterjira",
            url,
            &cfg,
            &["xdg-open".to_string(), "betterjira".to_string()]
        ));
        assert!(target_matches_ancestors(
            "custom:betterjira",
            url,
            &cfg,
            &["gio".to_string(), "betterjira-gui".to_string()]
        ));
        assert!(!target_matches_ancestors(
            "custom:betterjira",
            url,
            &cfg,
            &["xdg-open".to_string(), "t3-code".to_string()]
        ));
        assert!(!target_matches_ancestors(
            "chrome:Profile 2",
            url,
            &cfg,
            &["google-chrome".to_string()]
        ));
    }
}
