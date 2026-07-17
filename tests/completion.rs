//! Integration tests for dynamic shell completion (`COMPLETE=$shell`).

use assert_cmd::Command;
use predicates::prelude::*;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

/// `COMPLETE=bash recomplog` with no completion args prints the registration script.
#[test]
fn complete_bash_prints_registration() {
    bin()
        .env("COMPLETE", "bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("recomplog"))
        .stdout(
            predicate::str::contains("complete")
                .or(predicate::str::contains("_clap"))
                .or(predicate::str::contains("COMPREPLY")),
        );
}

/// Without COMPLETE, normal CLI still works (help).
#[test]
fn without_complete_env_normal_help_works() {
    bin()
        .args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("recomplog"))
        .stdout(predicate::str::contains("workout"));
}

/// Completing after the binary name should offer top-level subcommands.
#[test]
fn complete_bash_top_level_subcommands() {
    // Protocol: COMPLETE=bash <bin> -- <words...> with index env for some shells.
    // clap_complete bash uses _CLAP_COMPLETE_INDEX for the current word index.
    bin()
        .env("COMPLETE", "bash")
        .env("_CLAP_COMPLETE_INDEX", "1")
        .args(["--", "recomplog", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("workout"))
        .stdout(predicate::str::contains("body"))
        .stdout(predicate::str::contains("nutrition"));
}
