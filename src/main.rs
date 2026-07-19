mod bodyweight;
mod cli;
mod commands;
mod completion;
mod config;
mod db;
mod entity_audit;
mod error;
mod fit;
mod hr_zones;
mod infoods;
mod load_type;
mod macro_names;
mod models;
mod nutrition_units;
mod phase;
mod product_resolve;
mod repository;
mod sanity;
mod stats;
mod track_metrics;
mod utils;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;

use cli::Cli;

fn main() {
    // Dynamic shell completion: when COMPLETE=$shell is set, print registration
    // or candidates and exit. Must run before any stdout and before normal parse.
    CompleteEnv::with_factory(Cli::command).complete();

    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    commands::dispatch(cli)
}
