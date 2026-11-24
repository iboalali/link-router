use crate::config::load_config;
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DESKTOP_FILE_NAME: &str = "link-router.desktop";
pub const ICON_NAME: &str = "link-router";
pub const NATIVE_HOST_NAME: &str = "com.iboalali.link_router";
const ICON_SVG: &str = include_str!("../assets/link-router.svg");

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    pub register_default: bool,
    pub extension_ids: Vec<String>,
    pub binary_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallStatus {
    pub desktop_file: PathBuf,
    pub desktop_installed: bool,
    pub http_default: Option<String>,
    pub https_default: Option<String>,
    pub is_default_browser: bool,
    pub native_host_file: PathBuf,
    pub native_host_installed: bool,
}

pub fn desktop_file_path() -> PathBuf {
    data_home().join("applications").join(DESKTOP_FILE_NAME)
}

pub fn icon_file_path() -> PathBuf {
    data_home()
        .join("icons/hicolor/scalable/apps")
        .join(format!("{}.svg", ICON_NAME))
}

fn data_home() -> PathBuf {
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.trim().is_empty() {
            return PathBuf::from(xdg_data);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share")
}

pub fn native_host_manifest_path() -> PathBuf {
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg_config.trim().is_empty() {
            return PathBuf::from(xdg_config)
                .join("google-chrome/NativeMessagingHosts")
                .join(format!("{}.json", NATIVE_HOST_NAME));
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config/google-chrome/NativeMessagingHosts")
        .join(format!("{}.json", NATIVE_HOST_NAME))
}

pub fn install(opts: &InstallOptions) -> Result<Vec<String>, String> {
    let bin = match &opts.binary_path {
        Some(p) => p.clone(),
        None => std::env::current_exe()
            .map_err(|e| format!("failed to locate current binary: {}", e))?,
    };
    let bin_str = bin.to_string_lossy();
    let mut actions = Vec::new();

    let icon_path = icon_file_path();
    if let Some(parent) = icon_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }
    fs::write(&icon_path, ICON_SVG)
        .map_err(|e| format!("failed to write {}: {}", icon_path.display(), e))?;
    actions.push(format!("Wrote {}", icon_path.display()));
    let hicolor_dir = data_home().join("icons/hicolor");
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(&hicolor_dir)
        .status();

    let desktop_path = desktop_file_path();
    if let Some(parent) = desktop_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }
    let exec_bin = if bin_str.contains(' ') {
        format!("\"{}\"", bin_str)
    } else {
        bin_str.to_string()
    };
    let desktop_content = format!(
        "[Desktop Entry]\n\
         Version=1.0\n\
         Type=Application\n\
         Name=Link Router\n\
         Comment=Route HTTP and HTTPS links to apps and browser profiles\n\
         Exec={} open %u\n\
         Icon={}\n\
         Terminal=false\n\
         Categories=Network;WebBrowser;\n\
         MimeType=x-scheme-handler/http;x-scheme-handler/https;\n\
         StartupNotify=true\n",
        exec_bin, ICON_NAME
    );
    fs::write(&desktop_path, desktop_content)
        .map_err(|e| format!("failed to write {}: {}", desktop_path.display(), e))?;
    actions.push(format!("Wrote {}", desktop_path.display()));

    if let Some(apps_dir) = desktop_path.parent() {
        let _ = Command::new("update-desktop-database").arg(apps_dir).status();
    }

    if opts.register_default {
        run_xdg_mime_default(DESKTOP_FILE_NAME)?;
        actions.push("Registered link-router.desktop for http and https".to_string());
    }

    if !opts.extension_ids.is_empty() {
        let host_path = native_host_manifest_path();
        if let Some(parent) = host_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }
        let wrapper_path = write_native_host_wrapper(&bin)?;
        // Each Chrome profile can give the unpacked extension its own ID, so IDs
        // registered by earlier installs stay allowed.
        let mut origins = existing_allowed_origins(&host_path);
        for id in &opts.extension_ids {
            let origin = format!("chrome-extension://{}/", id.trim());
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
        let manifest = json!({
            "name": NATIVE_HOST_NAME,
            "description": "Link Router Chrome Native Messaging host",
            "path": wrapper_path.to_string_lossy(),
            "type": "stdio",
            "allowed_origins": origins
        });
        let rendered = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("failed to serialize native host manifest: {}", e))?;
        fs::write(&host_path, rendered)
            .map_err(|e| format!("failed to write {}: {}", host_path.display(), e))?;
        actions.push(format!(
            "Wrote {} (allowed origins: {})",
            host_path.display(),
            origins.join(", ")
        ));
    }

    Ok(actions)
}

fn existing_allowed_origins(host_path: &Path) -> Vec<String> {
    fs::read_to_string(host_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("allowed_origins").and_then(|o| o.as_array()).cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| o.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn write_native_host_wrapper(bin: &Path) -> Result<PathBuf, String> {
    let dir = crate::config::config_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {}", dir.display(), e))?;
    let wrapper = dir.join("native-host.sh");
    let script = format!("#!/bin/sh\nexec \"{}\" native-host\n", bin.to_string_lossy());
    fs::write(&wrapper, script)
        .map_err(|e| format!("failed to write {}: {}", wrapper.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("failed to chmod {}: {}", wrapper.display(), e))?;
    }
    Ok(wrapper)
}

pub fn uninstall() -> Result<Vec<String>, String> {
    let mut actions = Vec::new();
    let current = status();

    if current.is_default_browser {
        let fallback_desktop = load_config()
            .ok()
            .and_then(|c| {
                c.fallback_target
                    .strip_prefix("desktop:")
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "google-chrome.desktop".to_string());
        if fallback_desktop != DESKTOP_FILE_NAME {
            let _ = run_xdg_mime_default(&fallback_desktop);
            actions.push(format!(
                "Restored default http/https handler to {}",
                fallback_desktop
            ));
        }
    }

    let desktop_path = desktop_file_path();
    if desktop_path.exists() {
        fs::remove_file(&desktop_path)
            .map_err(|e| format!("failed to remove {}: {}", desktop_path.display(), e))?;
        actions.push(format!("Removed {}", desktop_path.display()));
        if let Some(apps_dir) = desktop_path.parent() {
            let _ = Command::new("update-desktop-database").arg(apps_dir).status();
        }
    }

    let icon_path = icon_file_path();
    if icon_path.exists() {
        fs::remove_file(&icon_path)
            .map_err(|e| format!("failed to remove {}: {}", icon_path.display(), e))?;
        actions.push(format!("Removed {}", icon_path.display()));
    }

    let host_path = native_host_manifest_path();
    if host_path.exists() {
        fs::remove_file(&host_path)
            .map_err(|e| format!("failed to remove {}: {}", host_path.display(), e))?;
        actions.push(format!("Removed {}", host_path.display()));
    }

    let wrapper = crate::config::config_dir().join("native-host.sh");
    if wrapper.exists() {
        let _ = fs::remove_file(&wrapper);
    }

    Ok(actions)
}

pub fn status() -> InstallStatus {
    let desktop_file = desktop_file_path();
    let native_host_file = native_host_manifest_path();
    let http_default = query_xdg_mime("x-scheme-handler/http");
    let https_default = query_xdg_mime("x-scheme-handler/https");
    let is_default_browser = http_default.as_deref() == Some(DESKTOP_FILE_NAME)
        && https_default.as_deref() == Some(DESKTOP_FILE_NAME);

    InstallStatus {
        desktop_installed: desktop_file.exists(),
        desktop_file,
        http_default,
        https_default,
        is_default_browser,
        native_host_installed: native_host_file.exists(),
        native_host_file,
    }
}

fn run_xdg_mime_default(desktop_id: &str) -> Result<(), String> {
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg_config.trim().is_empty() {
            let _ = fs::create_dir_all(&xdg_config);
        }
    }
    for mime in ["x-scheme-handler/http", "x-scheme-handler/https"] {
        let status = Command::new("xdg-mime")
            .args(["default", desktop_id, mime])
            .status()
            .map_err(|e| format!("failed to execute xdg-mime: {}", e))?;
        if !status.success() {
            return Err(format!("xdg-mime exited with status {} for {}", status, mime));
        }
    }
    Ok(())
}

fn query_xdg_mime(mime: &str) -> Option<String> {
    let out = Command::new("xdg-mime")
        .args(["query", "default", mime])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}
