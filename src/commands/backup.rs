//! `recomplog db backup` — copy the SQLite database file.

use crate::db;
use crate::utils::print_json;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};

/// Create a file copy of the database.
///
/// Source is the global `--db` path (or the XDG default). Destination defaults to a
/// timestamped sibling of the source; `--to` accepts a file or directory path.
pub fn handle(
    to: Option<&str>,
    force: bool,
    db_override: Option<&str>,
    json: bool,
    quiet: bool,
) -> Result<()> {
    let source = resolve_source_path(db_override);
    if !source.is_file() {
        bail!(
            "database not found at {} (run `recomplog init` or pass --db)",
            source.display()
        );
    }

    let destination = resolve_destination(&source, to)?;
    if paths_equal(&source, &destination)? {
        bail!(
            "destination is the same as the source database ({})",
            source.display()
        );
    }

    if destination.exists() {
        if destination.is_dir() {
            bail!(
                "destination exists and is a directory: {} (pass a file path or trailing slash for dir mode)",
                destination.display()
            );
        }
        if !force {
            bail!(
                "destination already exists: {} (use --force to overwrite)",
                destination.display()
            );
        }
    }

    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create destination directory {}",
                    parent.display()
                )
            })?;
        }
    }

    let bytes = fs::copy(&source, &destination).with_context(|| {
        format!(
            "failed to copy {} → {}",
            source.display(),
            destination.display()
        )
    })?;

    if json {
        print_json(&serde_json::json!({
            "success": true,
            "source": source.display().to_string(),
            "destination": destination.display().to_string(),
            "bytes": bytes,
            "message": "database backed up",
        }));
    } else if !quiet {
        println!("Backed up {}", source.display());
        println!("     → {}", destination.display());
    }

    Ok(())
}

/// Path only — no directory creation, no open.
fn resolve_source_path(override_path: Option<&str>) -> PathBuf {
    match override_path {
        Some(p) => PathBuf::from(p),
        None => db::default_db_path(),
    }
}

/// Build destination path from optional `--to` and the source file.
fn resolve_destination(source: &Path, to: Option<&str>) -> Result<PathBuf> {
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let default_name = default_backup_filename(source, &stamp.to_string());

    match to {
        None => {
            let parent = source.parent().unwrap_or_else(|| Path::new("."));
            Ok(parent.join(default_name))
        }
        Some(raw) => {
            let path = PathBuf::from(raw);
            // Trailing slash → treat as directory even if it does not exist yet.
            let as_dir = raw.ends_with('/')
                || raw.ends_with(std::path::MAIN_SEPARATOR)
                || (path.exists() && path.is_dir());
            if as_dir {
                Ok(path.join(default_name))
            } else {
                Ok(path)
            }
        }
    }
}

fn default_backup_filename(source: &Path, stamp: &str) -> String {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recomplog");
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("db");
    format!("{stem}-{stamp}.{ext}")
}

fn paths_equal(a: &Path, b: &Path) -> Result<bool> {
    // Best-effort: canonicalize when both exist; otherwise compare as given.
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => Ok(ca == cb),
        _ => Ok(a == b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filename_uses_stem_and_ext() {
        let name = default_backup_filename(Path::new("/x/recomplog.db"), "20260718T120000Z");
        assert_eq!(name, "recomplog-20260718T120000Z.db");
    }

    #[test]
    fn destination_default_is_sibling() {
        let src = PathBuf::from("/data/recomplog.db");
        let dest = resolve_destination(&src, None).unwrap();
        assert_eq!(dest.parent(), Some(Path::new("/data")));
        assert!(dest
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("recomplog-"));
        assert!(dest.extension().and_then(|e| e.to_str()) == Some("db"));
    }

    #[test]
    fn destination_to_file() {
        let src = PathBuf::from("/data/recomplog.db");
        let dest = resolve_destination(&src, Some("/tmp/out.db")).unwrap();
        assert_eq!(dest, PathBuf::from("/tmp/out.db"));
    }

    #[test]
    fn destination_to_dir_trailing_slash() {
        let src = PathBuf::from("/data/recomplog.db");
        let dest = resolve_destination(&src, Some("/tmp/backups/")).unwrap();
        assert_eq!(dest.parent(), Some(Path::new("/tmp/backups")));
        assert!(dest
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("recomplog-"));
    }
}
