mod config;
mod engine;
mod install;
mod native_host;
mod targets;
mod ui;

use clap::{Parser, Subcommand};
use config::{load_config, load_handler_manifests, save_config};
use engine::{append_builtin_bypass_param, resolve, Decision, ResolveOptions};
use install::InstallOptions;
use std::path::PathBuf;
use std::process::ExitCode;
use targets::{ancestor_executable_names, discover_targets, launch_target, target_matches_ancestors};
use ui::chooser::{run_chooser, SaveScope};

#[derive(Parser)]
#[command(name = "link-router", version, about = "Android-style URL router for Linux and Chrome")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Route and open an HTTP/HTTPS URL
    Open {
        /// Force the "Open with" chooser popup even when an Always rule matches
        #[arg(long)]
        ask: bool,
        /// Optional source target id (for example chrome:Default)
        #[arg(long)]
        source: Option<String>,
        /// URL to open
        url: String,
    },
    /// Open the GTK4/libadwaita settings and rule manager
    Manager,
    /// Run as a Chrome Native Messaging host over stdin/stdout
    NativeHost,
    /// Dry-run URL resolution and print the matching trace
    Test {
        #[arg(long)]
        ask: bool,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        json: bool,
        url: String,
    },
    /// List discovered browser profiles, desktop apps, and custom targets
    Targets {
        #[arg(long)]
        json: bool,
    },
    /// Install desktop entry and optional Chrome Native Messaging manifest
    Install {
        /// Also register link-router.desktop as the default http/https handler
        #[arg(long)]
        register_default: bool,
        /// Chrome extension ID to allow in NativeMessagingHosts (repeatable; IDs already
        /// allowed stay allowed)
        #[arg(long = "extension-id")]
        extension_ids: Vec<String>,
        /// Register this binary as-is instead of copying the running one to
        /// ~/.local/bin/link-router
        #[arg(long)]
        binary_path: Option<PathBuf>,
    },
    /// Remove desktop entry, Native Messaging manifest, and restore fallback default
    Uninstall,
    /// Print desktop and Chrome integration status
    Status {
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        None | Some(Commands::Manager) => {
            ui::manager::run_manager();
            Ok(())
        }
        Some(Commands::Open { ask, source, url }) => cmd_open(&url, ask, source.as_deref()),
        Some(Commands::NativeHost) => native_host::run_stdio_loop(),
        Some(Commands::Test {
            ask,
            source,
            json,
            url,
        }) => cmd_test(&url, ask, source.as_deref(), json),
        Some(Commands::Targets { json }) => cmd_targets(json),
        Some(Commands::Install {
            register_default,
            extension_ids,
            binary_path,
        }) => cmd_install(InstallOptions {
            register_default,
            extension_ids,
            binary_path,
        }),
        Some(Commands::Uninstall) => cmd_uninstall(),
        Some(Commands::Status { json }) => cmd_status(json),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("link-router: {}", err);
            ExitCode::FAILURE
        }
    }
}

fn cmd_open(url: &str, force_ask: bool, source: Option<&str>) -> Result<(), String> {
    let mut cfg = load_config()?;
    let manifests = load_handler_manifests();

    let res = resolve(
        url,
        &cfg,
        &manifests,
        ResolveOptions {
            force_ask,
            source_target: source,
        },
    )?;

    match res.decision {
        Decision::AllowInSource { .. } => Ok(()),
        Decision::Open { target } => {
            if cfg.detect_self_loop {
                let ancestors = ancestor_executable_names();
                if target_matches_ancestors(&target, &res.effective_url, &cfg, &ancestors) {
                    let bypass_url = append_builtin_bypass_param(&res.effective_url);
                    return launch_target(&cfg.fallback_target, &bypass_url, &cfg);
                }
            }
            launch_target(&target, &res.effective_url, &cfg)
        }
        Decision::Ask {
            candidates,
            suggested_host,
            suggested_path_prefix,
        } => {
            let all_targets = discover_targets(&cfg);
            let Some(choice) = run_chooser(
                &res.effective_url,
                &suggested_host,
                suggested_path_prefix.as_deref(),
                &candidates,
                &all_targets,
            ) else {
                return Ok(());
            };

            if let Some(scope) = choice.save_scope {
                let prefix = match scope {
                    SaveScope::HostOnly => None,
                    SaveScope::HostAndPath => suggested_path_prefix.as_deref(),
                };
                cfg.upsert_always_rule(&suggested_host, prefix, &choice.target_id);
                save_config(&cfg)?;
            }

            launch_target(&choice.target_id, &res.effective_url, &cfg)
        }
    }
}

fn cmd_test(url: &str, force_ask: bool, source: Option<&str>, as_json: bool) -> Result<(), String> {
    let cfg = load_config()?;
    let manifests = load_handler_manifests();
    let res = resolve(
        url,
        &cfg,
        &manifests,
        ResolveOptions {
            force_ask,
            source_target: source,
        },
    )?;

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&res).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    println!("Original URL:  {}", res.original_url);
    println!("Effective URL: {}", res.effective_url);
    match &res.decision {
        Decision::Open { target } => println!("Decision:      open -> {}", target),
        Decision::AllowInSource { target } => {
            println!("Decision:      allow in source -> {}", target)
        }
        Decision::Ask {
            candidates,
            suggested_host,
            suggested_path_prefix,
        } => {
            println!("Decision:      ask");
            println!("Candidates:    {}", candidates.join(", "));
            println!("Host:          {}", suggested_host);
            if let Some(p) = suggested_path_prefix {
                println!("Path prefix:   {}", p);
            }
        }
    }
    println!("Trace:");
    for line in &res.trace {
        println!("  - {}", line);
    }
    Ok(())
}

fn cmd_targets(as_json: bool) -> Result<(), String> {
    let cfg = load_config()?;
    let targets = discover_targets(&cfg);
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&targets).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    for t in targets {
        println!("{:<42} | {:<28} | {}", t.id, t.name, t.subtitle);
    }
    Ok(())
}

fn cmd_install(opts: InstallOptions) -> Result<(), String> {
    let actions = install::install(&opts)?;
    for a in actions {
        println!("{}", a);
    }
    Ok(())
}

fn cmd_uninstall() -> Result<(), String> {
    let actions = install::uninstall()?;
    if actions.is_empty() {
        println!("Nothing to uninstall.");
    } else {
        for a in actions {
            println!("{}", a);
        }
    }
    Ok(())
}

fn cmd_status(as_json: bool) -> Result<(), String> {
    let st = install::status();
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&st).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    println!("Config:       {}", config::config_path().display());
    println!(
        "Desktop file: {} ({})",
        st.desktop_file.display(),
        if st.desktop_installed {
            "installed"
        } else {
            "missing"
        }
    );
    println!(
        "HTTP default: {}",
        st.http_default.as_deref().unwrap_or("unset")
    );
    println!(
        "HTTPS default:{}",
        st.https_default.as_deref().unwrap_or("unset")
    );
    println!(
        "Native host:  {} ({})",
        st.native_host_file.display(),
        if st.native_host_installed {
            "installed"
        } else {
            "missing"
        }
    );
    Ok(())
}
