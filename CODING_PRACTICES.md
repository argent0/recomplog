# Coding Practices for recomplog

This document defines the coding standards for the unified `recomplog` project.

## Goals

- Pleasant for humans *and* LLM agents.
- Predictable, well-documented, scriptable CLI.
- High-quality Rust without over-engineering.

## Core Principles

1. **LLM-Agent Friendly First**
   - `--json` on every command that returns data.
   - Consistent `entity action` pattern.
   - Clear actionable errors.
   - Excellent help text.

2. **Simplicity over Cleverness**
   - Single-user local SQLite tool.
   - Prefer `rusqlite` + explicit queries.
   - `clap` derive for CLI.
   - `thiserror` for domain errors, `anyhow` at the binary boundary.

3. **Formatting & Style**
   - `cargo fmt` before every commit (respects `rustfmt.toml`).
   - Max width 100.
   - 4-space indent.

4. **Linting**
   - `cargo clippy -- -D warnings` before committing.
   - See `clippy.toml`.

5. **Error Handling**
   - No `unwrap()`, `expect()`, or `panic!` in production paths.
   - Never silently drop errors.

6. **Database**
   - All changes via migrations in `migrations/`.
   - Foreign keys on.
   - **Two clocks:** event time (when the user says it happened) vs storage time
     (`created_at` / `updated_at` / `imported_at` = always `now_utc()`; never user input).
   - Instants: UTC RFC3339 (`YYYY-MM-DDTHH:MM:SSZ`) only — write via `format_instant_utc` /
     `now_utc` / `parse_rfc3339_instant_for_db` + `validate_instant_for_db`.
   - Calendar days: `YYYY-MM-DD` only (event day for measurements/sleep).
   - Day buckets and reports use **event** fields only, never `created_at`.
   - Never rely on SQLite `datetime('now')` for new rows; always set timestamps from Rust.
   - Keep the model pragmatic.

7. **Testing**
   - Unit tests for logic.
   - Integration tests via `assert_cmd`.
   - Fast tests.

8. **Documentation**
   - Public items get doc comments.
   - CLI help is primary docs.
   - Detailed guides live in `docs/`.
   - Update `CODING_PRACTICES.md` when conventions change.

## Recommended Workflow

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

## References

- The AGENTS.md in this repo
- Original tool docs from repslog, bodylog, nutlog (for behavioral compatibility)
