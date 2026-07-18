# Coding Practices for recomplog

This document defines the coding standards for the unified `recomplog` project.

## Goals

- Pleasant for humans *and* LLM agents.
- Predictable, well-documented, scriptable CLI.
- High-quality Rust without over-engineering.
- Quality data → quality reports → actionable reports (see `AGENTS.md` philosophy).
- **Append only — no exceptions. Never, nowhere.** (see `AGENTS.md` philosophy).

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

3. **Append only — no exceptions. Never, nowhere.**
   - Event history grows **only** by insert (`create` / `add` / import). No domain,
     feature, migration, import flag, or “agent convenience” may rewrite settled
     event rows. If you need a different past, append a new fact (or supersede/void
     when that model exists) — do not UPDATE/DELETE history as the product path.
   - Catalog/config may update or merge; that is not event history.
   - Imports append and stay idempotent; never replace a domain’s history as a side effect.
   - Keep event time vs storage time distinct when appending late entries.
   - No day-level uniqueness on event logs (measurements/sleep multi-sample; same for
     any future event table). Day aggregation for reports is read-side only.
   - Log `update`/`delete` that still exist are legacy correction debt — do not build
     new features on them; do not teach agents to upsert events.
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
   - Calendar days: `YYYY-MM-DD` only (event day for measurements/sleep). Multiple
     measurement/sleep rows may share the same event day.
   - Day buckets and reports use **event** fields only, never `created_at` for bucketing;
     when collapsing multi-sample days, prefer last-by-`created_at` (then `id`).
   - Never rely on SQLite `datetime('now')` for new rows; always set timestamps from Rust.
   - Event tables must be append-friendly: no uniqueness that forces rewrite of
     history (e.g. no `UNIQUE` on event calendar day).

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
