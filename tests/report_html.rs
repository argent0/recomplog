use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("recomplog").unwrap()
}

#[test]
fn report_html_writes_file() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.db").display().to_string();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    bin()
        .args([
            "--db",
            &db,
            "body",
            "measurement",
            "create",
            "--date",
            "today",
            "--weight-kg",
            "80",
        ])
        .assert()
        .success();

    bin()
        .args([
            "--db",
            &db,
            "--json",
            "report",
            "html",
            "--days",
            "7",
            "--output-dir",
            &out.display().to_string(),
            "--name",
            "dash.html",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\": true"));

    let html = std::fs::read_to_string(out.join("dash.html")).unwrap();
    assert!(html.contains("recomplog"));
    assert!(html.contains("Chart"));
}
