use crate::config::{load_config, load_handler_manifests, save_config, Config, HandlerManifest};
use crate::engine::{export_extension_rules, resolve, Decision, ResolveOptions};
use crate::targets::{discover_chrome_profiles, discover_targets, launch_target};
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};

const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

pub fn read_frame<R: Read>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("native messaging payload too large: {} bytes", len),
        ));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

pub fn run_stdio_loop() -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    while let Some(frame) =
        read_frame(&mut reader).map_err(|e| format!("failed to read native frame: {}", e))?
    {
        let response = match serde_json::from_slice::<Value>(&frame) {
            Ok(req) => {
                let mut config = load_config().unwrap_or_default();
                let manifests = load_handler_manifests();
                handle_request(&req, &mut config, &manifests, false)
            }
            Err(e) => json!({ "ok": false, "error": format!("invalid JSON: {}", e) }),
        };
        let bytes = serde_json::to_vec(&response)
            .map_err(|e| format!("failed to serialize native response: {}", e))?;
        write_frame(&mut writer, &bytes)
            .map_err(|e| format!("failed to write native frame: {}", e))?;
    }

    Ok(())
}

pub fn handle_request(
    req: &Value,
    config: &mut Config,
    manifests: &[HandlerManifest],
    dry_run: bool,
) -> Value {
    let req_type = req.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match req_type {
        "ping" => json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
        }),
        "get_state" => {
            let profiles = discover_chrome_profiles();
            let targets = discover_targets(config);
            let rules = export_extension_rules(config, manifests);
            json!({
                "ok": true,
                "fallback_target": config.fallback_target,
                "unwrap_redirects": config.unwrap_redirects,
                "bypass_query_params": config.bypass_query_params,
                "profiles": profiles,
                "targets": targets,
                "rules": rules,
            })
        }
        "open_manager" => {
            if !dry_run {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = Command::new(exe)
                        .arg("manager")
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn();
                }
            }
            json!({ "ok": true })
        }
        "route" => handle_route(req, config, manifests, dry_run),
        other => json!({
            "ok": false,
            "error": format!("unknown request type '{}'", other),
        }),
    }
}

fn handle_route(
    req: &Value,
    config: &mut Config,
    manifests: &[HandlerManifest],
    dry_run: bool,
) -> Value {
    let Some(url) = req.get("url").and_then(|v| v.as_str()) else {
        return json!({ "ok": false, "error": "missing 'url' field" });
    };
    let source_profile = req
        .get("source_profile")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.contains(':') {
                s.to_string()
            } else {
                format!("chrome:{}", s)
            }
        });
    let force_ask = req
        .get("force_ask")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let res = match resolve(
        url,
        config,
        manifests,
        ResolveOptions {
            force_ask,
            source_target: source_profile.as_deref(),
        },
    ) {
        Ok(r) => r,
        Err(e) => return json!({ "ok": false, "error": e }),
    };

    match res.decision {
        Decision::AllowInSource { target } => json!({
            "ok": true,
            "action": "allow",
            "target": target,
            "effective_url": res.effective_url,
        }),
        Decision::Open { target } => {
            if !dry_run {
                if let Err(e) = launch_target(&target, &res.effective_url, config) {
                    return json!({ "ok": false, "error": e });
                }
            }
            json!({
                "ok": true,
                "action": "redirected",
                "target": target,
                "effective_url": res.effective_url,
            })
        }
        Decision::Ask {
            candidates,
            suggested_host,
            suggested_path_prefix,
        } => {
            if dry_run {
                return json!({
                    "ok": true,
                    "action": "ask",
                    "candidates": candidates,
                    "effective_url": res.effective_url,
                });
            }
            let all_targets = discover_targets(config);
            let outcome = crate::ui::chooser::run_chooser(
                &res.effective_url,
                &suggested_host,
                suggested_path_prefix.as_deref(),
                &candidates,
                &all_targets,
            );
            match outcome {
                Some(choice) => {
                    if let Some(scope) = choice.save_scope {
                        let prefix = match scope {
                            crate::ui::chooser::SaveScope::HostOnly => None,
                            crate::ui::chooser::SaveScope::HostAndPath => {
                                suggested_path_prefix.as_deref()
                            }
                        };
                        config.upsert_always_rule(&suggested_host, prefix, &choice.target_id);
                        let _ = save_config(config);
                    }
                    if source_profile.as_deref() == Some(choice.target_id.as_str()) {
                        json!({
                            "ok": true,
                            "action": "allow",
                            "target": choice.target_id,
                            "effective_url": res.effective_url,
                        })
                    } else if let Err(e) =
                        launch_target(&choice.target_id, &res.effective_url, config)
                    {
                        json!({ "ok": false, "error": e })
                    } else {
                        json!({
                            "ok": true,
                            "action": "redirected",
                            "target": choice.target_id,
                            "effective_url": res.effective_url,
                        })
                    }
                }
                None => json!({
                    "ok": true,
                    "action": "canceled",
                    "effective_url": res.effective_url,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Rule, RuleAction};
    use std::io::Cursor;

    #[test]
    fn native_messaging_frame_round_trip() {
        let mut buf = Vec::new();
        let payload = br#"{"type":"ping"}"#;
        write_frame(&mut buf, payload).unwrap();

        let mut cursor = Cursor::new(buf);
        let read_back = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(read_back, payload);
        assert!(read_frame(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn route_request_allows_same_profile_and_redirects_other_profile() {
        let mut cfg = Config::default();
        cfg.fallback_target = "chrome:Default".to_string();
        cfg.rules.push(Rule {
            id: "work-jira".to_string(),
            name: "Work Jira".to_string(),
            enabled: true,
            host: "*.atlassian.net".to_string(),
            path_prefix: None,
            path_glob: None,
            action: RuleAction::Always,
            target: Some("chrome:Profile 1".to_string()),
            candidates: Vec::new(),
        });

        let allow = handle_request(
            &json!({
                "type": "route",
                "url": "https://acme.atlassian.net/browse/TM-10",
                "source_profile": "Profile 1"
            }),
            &mut cfg,
            &[],
            true,
        );
        assert_eq!(allow["ok"], true);
        assert_eq!(allow["action"], "allow");

        let redirect = handle_request(
            &json!({
                "type": "route",
                "url": "https://acme.atlassian.net/browse/TM-10",
                "source_profile": "Default"
            }),
            &mut cfg,
            &[],
            true,
        );
        assert_eq!(redirect["ok"], true);
        assert_eq!(redirect["action"], "redirected");
        assert_eq!(redirect["target"], "chrome:Profile 1");
    }
}
