//! Top-level config file management (sanity limits).

use crate::cli::ConfigAction;
use crate::config::{self, AppConfig};
use crate::models::Success;
use crate::utils::print_json;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Line appended to `~/.bashrc` for dynamic bash completion.
const BASH_COMPLETION_LINE: &str = "source <(COMPLETE=bash recomplog)";

pub fn handle(action: ConfigAction, config_override: Option<&str>, json: bool) -> Result<()> {
    match action {
        ConfigAction::Path => {
            let path = config::resolve_config_path(config_override);
            if json {
                print_json(&serde_json::json!({"path": path.display().to_string()}));
            } else {
                println!("{}", path.display());
            }
            Ok(())
        }
        ConfigAction::Show => {
            let loaded = config::load_or_create(config_override)?;
            if json {
                print_json(&loaded.config);
            } else {
                println!("Config: {}", loaded.path.display());
                println!("{}", toml::to_string_pretty(&loaded.config)?);
            }
            Ok(())
        }
        ConfigAction::Generate { path, force } => {
            let target = path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| config::resolve_config_path(config_override));
            if target.exists() && !force {
                return Err(anyhow::anyhow!(
                    "config already exists at {} (use --force to overwrite)",
                    target.display()
                ));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let cfg = AppConfig::default();
            fs::write(&target, toml::to_string_pretty(&cfg)?)?;
            if json {
                print_json(&Success::ok(format!(
                    "wrote default config to {}",
                    target.display()
                )));
            } else {
                println!("Wrote default config to {}", target.display());
            }
            Ok(())
        }
        ConfigAction::BashCompletion => handle_bash_completion(json),
    }
}

fn handle_bash_completion(json: bool) -> Result<()> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    let bashrc = PathBuf::from(home).join(".bashrc");
    let already = if bashrc.exists() {
        let contents = fs::read_to_string(&bashrc)
            .with_context(|| format!("failed to read {}", bashrc.display()))?;
        contents
            .lines()
            .any(|line| line.contains("COMPLETE=bash recomplog"))
    } else {
        false
    };

    if already {
        if json {
            print_json(&serde_json::json!({
                "success": true,
                "path": bashrc.display().to_string(),
                "changed": false,
                "message": "bash completion already configured",
            }));
        } else {
            println!(
                "Bash completion already configured in {}.",
                bashrc.display()
            );
        }
        return Ok(());
    }

    let mut contents = if bashrc.exists() {
        fs::read_to_string(&bashrc)
            .with_context(|| format!("failed to read {}", bashrc.display()))?
    } else {
        String::new()
    };
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str("# recomplog shell completion\n");
    contents.push_str(BASH_COMPLETION_LINE);
    contents.push('\n');
    fs::write(&bashrc, &contents)
        .with_context(|| format!("failed to write {}", bashrc.display()))?;

    if json {
        print_json(&serde_json::json!({
            "success": true,
            "path": bashrc.display().to_string(),
            "changed": true,
            "line": BASH_COMPLETION_LINE,
            "message": "appended bash completion to ~/.bashrc",
        }));
    } else {
        println!("Appended bash completion to {}.", bashrc.display());
        println!("Run: source ~/.bashrc  (or open a new terminal)");
    }
    Ok(())
}
