mod bodyweight;
mod cli;
mod commands;
mod config;
mod db;
mod error;
mod fit;
mod hr_zones;
mod load_type;
mod models;
mod nutrition_units;
mod phase;
mod repository;
mod sanity;
mod stats;
mod track_metrics;
mod utils;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    commands::dispatch(cli)
}
