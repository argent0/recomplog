//! `recomplog config bash-completion` writes bashrc setup.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

#[test]
fn config_bash_completion_appends_bashrc() {
    let home = TempDir::new().unwrap();
    let bashrc = home.path().join(".bashrc");
    std::fs::write(&bashrc, "# existing\n").unwrap();

    bin()
        .env("HOME", home.path())
        .args(["--json", "config", "bash-completion"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"changed\": true"))
        .stdout(predicate::str::contains("COMPLETE=bash recomplog"));

    let text = std::fs::read_to_string(&bashrc).unwrap();
    assert!(text.contains("source <(COMPLETE=bash recomplog)"));
    assert!(text.contains("# existing"));
}

#[test]
fn config_bash_completion_idempotent() {
    let home = TempDir::new().unwrap();
    let bashrc = home.path().join(".bashrc");
    std::fs::write(&bashrc, "source <(COMPLETE=bash recomplog)\n").unwrap();

    bin()
        .env("HOME", home.path())
        .args(["--json", "config", "bash-completion"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"changed\": false"));

    let text = std::fs::read_to_string(&bashrc).unwrap();
    assert_eq!(text.matches("COMPLETE=bash recomplog").count(), 1);
}
