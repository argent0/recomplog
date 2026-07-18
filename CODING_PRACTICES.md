# Coding Practices for recomplog

This document defines the coding standards for the unified `recomplog` project.

## Goals

- Pleasant for humans *and* LLM agents.
- Predictable, well-documented, scriptable CLI.
- High-quality Rust without over-engineering.
- Quality data → quality reports → actionable reports (see `AGENTS.md` philosophy).
- Append-only event history: prefer insert over rewrite (see `AGENTS.md` philosophy).

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

3. **Append-only event history**
   - Default write path for logs is insert (`create` / `add` / import).
   - Catalog/config rows may update; event rows should not be the normal place to “fix” history in bulk.
   - Design new domain concepts so corrections do not require silent rewrites of past events.
   - Imports append and stay idempotent where possible; never replace an entire domain’s history as a side effect.
   - Keep event time vs storage time distinct when appending late entries.
   - **Consumption quantity/unit:** canonicalize at insert only. Do not re-run
     migration heuristics (`normalize_nutrition_units`, `promote_whole_package_products`)
     on open, import, or as a silent repair path. Those run once under `user_version`
     gates; further unit fixes are explicit user/agent corrections, not migrate-on-open.

4. **Data quality and actionable reports**
   - Protect event-time integrity and refuse garbage on write when possible.
   - Reports should surface what to do next (gaps, trends, summaries), not only raw dumps.
   - Prefer decision-ready output over decorative metrics.

5. **Formatting & Style**
   - `cargo fmt` before every commit (respects `rustfmt.toml`).
   - Max width 100.
   - 4-space indent.

6. **Linting**
   - `cargo clippy -- -D warnings` before committing.
   - See `clippy.toml`.

7. **Error Handling**
   - No `unwrap()`, `expect()`, or `panic!` in production paths.
   - Never silently drop errors.

8. **Database**
   - All changes via migrations in `migrations/`.
   - Foreign keys on.
   - **Two clocks:** event time (when the user says it happened) vs storage time
     (`created_at` / `updated_at` / `imported_at` = always `now_utc()`; never user input).
   - Instants: UTC RFC3339 (`YYYY-MM-DDTHH:MM:SSZ`) only — write via `format_instant_utc` /
     `now_utc` / `parse_rfc3339_instant_for_db` + `validate_instant_for_db`.
   - Calendar days: `YYYY-MM-DD` only (event day for measurements/sleep).
   - Day buckets and reports use **event** fields only, never `created_at`.
   - Never rely on SQLite `datetime('now')` for new rows; always set timestamps from Rust.
   - Keep the model pragmatic; prefer append-friendly schemas for event tables.

9. **Testing**
   - Unit tests for logic.
   - Integration tests via `assert_cmd`.
   - Fast tests.

10. **Documentation**
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
