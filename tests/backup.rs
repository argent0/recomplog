//! Integration tests for `recomplog db backup`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

fn seed_db(dir: &TempDir) -> String {
    let db = dir.path().join("recomplog.db").display().to_string();
    bin().args(["--db", &db, "init"]).assert().success();
    db
}

#[test]
fn backup_default_creates_timestamped_sibling() {
    let dir = TempDir::new().unwrap();
    let db = seed_db(&dir);
    let src_size = fs::metadata(&db).unwrap().len();

    bin()
        .args(["--db", &db, "--json", "db", "backup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"))
        .stdout(predicate::str::contains("\"destination\""))
        .stdout(predicate::str::contains("database backed up"));

    let backups: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("recomplog-") && n.ends_with(".db"))
        })
        .collect();
    assert_eq!(
        backups.len(),
        1,
        "expected one timestamped backup, got {backups:?}"
    );
    assert_eq!(fs::metadata(&backups[0]).unwrap().len(), src_size);
}

#[test]
fn backup_to_file_path() {
    let dir = TempDir::new().unwrap();
    let db = seed_db(&dir);
    let dest = dir.path().join("copy.db");

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "db",
            "backup",
            "--to",
            dest.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("copy.db"));

    assert!(dest.is_file());
    assert_eq!(
        fs::metadata(&db).unwrap().len(),
        fs::metadata(&dest).unwrap().len()
    );
}

#[test]
fn backup_to_directory() {
    let dir = TempDir::new().unwrap();
    let db = seed_db(&dir);
    let backup_dir = dir.path().join("backups");
    fs::create_dir(&backup_dir).unwrap();
    // Trailing slash so non-existent dirs would also work; here dir exists.
    let to = format!("{}/", backup_dir.display());

    bin()
        .args(["--db", &db, "--json", "db", "backup", "--to", &to])
        .assert()
        .success();

    let entries: Vec<_> = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert_eq!(entries.len(), 1);
    assert!(entries[0]
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("recomplog-"));
}

#[test]
fn backup_refuse_overwrite_without_force() {
    let dir = TempDir::new().unwrap();
    let db = seed_db(&dir);
    let dest = dir.path().join("existing.db");
    fs::write(&dest, b"old").unwrap();

    bin()
        .args(["--db", &db, "db", "backup", "--to", dest.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    assert_eq!(fs::read(&dest).unwrap(), b"old");
}

#[test]
fn backup_force_overwrites() {
    let dir = TempDir::new().unwrap();
    let db = seed_db(&dir);
    let dest = dir.path().join("existing.db");
    fs::write(&dest, b"old").unwrap();
    let src_size = fs::metadata(&db).unwrap().len();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "db",
            "backup",
            "--to",
            dest.to_str().unwrap(),
            "--force",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    assert_eq!(fs::metadata(&dest).unwrap().len(), src_size);
    assert_ne!(fs::read(&dest).unwrap(), b"old");
}

#[test]
fn backup_missing_source_fails() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("missing.db").display().to_string();

    bin()
        .args(["--db", &db, "db", "backup"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("database not found"));
}

#[test]
fn backup_self_copy_refused() {
    let dir = TempDir::new().unwrap();
    let db = seed_db(&dir);

    bin()
        .args(["--db", &db, "db", "backup", "--to", &db])
        .assert()
        .failure()
        .stderr(predicate::str::contains("same as the source"));
}

#[test]
fn backup_creates_parent_dirs_for_to() {
    let dir = TempDir::new().unwrap();
    let db = seed_db(&dir);
    let dest = dir.path().join("nested/deep/out.db");

    bin()
        .args(["--db", &db, "db", "backup", "--to", dest.to_str().unwrap()])
        .assert()
        .success();

    assert!(Path::new(&dest).is_file());
}
