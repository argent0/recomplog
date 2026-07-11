//! Command handlers for recomplog (grouped CLI surface).

pub mod body;
mod config_cmd;
mod import;
mod nutrition;
mod report;
mod workout;

use crate::cli::{Cli, Commands};
use crate::config;
use crate::db;
use crate::repository::BodyRepository;
use anyhow::Result;

pub fn dispatch(cli: Cli) -> Result<()> {
    let db_override = cli.db.as_deref();
    let json = cli.json;
    let quiet = cli.quiet;

    // Load config (create defaults if missing)
    let loaded = config::load_or_create(cli.config.as_deref())?;
    if loaded.created && !quiet {
        eprintln!(
            "Created default config at {} (edit to adjust sanity limits)",
            loaded.path.display()
        );
    }
    let sanity = &loaded.config.sanity;

    match cli.command {
        Commands::Version => {
            println!("recomplog {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Commands::Init { dry_run } => {
            if dry_run {
                if json {
                    println!(
                        r#"{{"success":true,"message":"dry-run: would initialize database and seed exercises"}}"#
                    );
                } else {
                    println!(
                        "dry-run: would create/open database, apply migrations, seed exercises"
                    );
                }
                return Ok(());
            }
            let conn = db::open_db(db_override)?;
            let added = workout::seed_default_exercises(&conn)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "success": true,
                        "message": "database initialized",
                        "added_exercises": added,
                    }))?
                );
            } else {
                println!("Database initialized (or already up to date).");
                for n in &added {
                    println!("  seeded exercise: {n}");
                }
            }
            Ok(())
        }
        Commands::Migrate {
            status,
            dry_run,
            force: _,
        } => {
            let target = 2;
            if status || dry_run {
                if json {
                    println!(
                        r#"{{"current":2,"latest":{},"dry_run":{}}}"#,
                        target, dry_run
                    );
                } else {
                    println!("Schema target version: {target} (applied automatically on open)");
                }
                return Ok(());
            }
            let _conn = db::open_db(db_override)?;
            println!("Migrations are applied automatically when opening the database.");
            Ok(())
        }
        Commands::Import { action } => import::handle(action, db_override, json),
        Commands::Check(args) => {
            let conn = db::open_db(db_override)?;
            let mut repo = BodyRepository::new(conn);
            body::handle_check(&mut repo, args, sanity, json, quiet).map_err(Into::into)
        }
        Commands::Workout { action } => {
            workout::handle(action, db_override, &sanity.workout, json, quiet)
        }
        Commands::Body { action } => {
            let conn = db::open_db(db_override)?;
            let mut repo = BodyRepository::new(conn);
            match action {
                crate::cli::BodyAction::Measurement { action } => {
                    body::handle_measurement(&mut repo, action, sanity, json, quiet)
                        .map_err(Into::into)
                }
                crate::cli::BodyAction::Sleep { action } => {
                    body::handle_sleep(&mut repo, *action, sanity, json, quiet).map_err(Into::into)
                }
                crate::cli::BodyAction::Profile { action } => {
                    body::handle_profile(&mut repo, action, json, quiet).map_err(Into::into)
                }
            }
        }
        Commands::Nutrition { action } => nutrition::handle(action, db_override, json, quiet),
        Commands::Report { action } => report::handle(action, db_override, sanity, json, quiet),
        Commands::Config { action } => config_cmd::handle(action, cli.config.as_deref(), json),
    }
}
