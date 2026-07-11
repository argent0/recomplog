//! Top-level config file management (sanity limits).

use crate::cli::ConfigAction;
use crate::config::{self, AppConfig};
use crate::models::Success;
use crate::utils::print_json;
use anyhow::Result;
use std::fs;

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
    }
}
