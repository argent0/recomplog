# AGENTS.md — Guidelines for LLM Agents Working on recomplog

`recomplog` is the unified successor to `repslog`, `bodylog`, `nutlog`, and `bodydashboard`.

It is a **single-user, local-first, LLM-agent-first** CLI tool for body recomposition tracking: strength & cardio workouts, body measurements, sleep, nutrition logging, and HTML reports.

## Core Philosophy (inherited and extended)

- Simplicity first — boring, maintainable, explicit.
- Agent-friendly by design — consistent `entity action`, excellent `--json`, predictable behavior.
- One database for everything (no more fragile subprocess + JSON glue between tools).
- Preserve the spirit of the original four tools.

## How to Work as an Agent

1. **Start here**: Read `AGENTS.md`, `CODING_PRACTICES.md`, `README.md`, and relevant `docs/`.
2. Use the standard command shape: `recomplog <group> <entity> <action> ...`
   - Training: `workout create|list|show|delete`, `workout exercise ...`, `workout set add|add-cardio|delete`
   - Body: `body measurement ...`, `body sleep ...`, `body profile ...`
   - Nutrition: `nutrition product|purchase|consumption|nutrient ...`
   - Cross-cutting: `report` (including `report brief`), `import`, `config`, `check` (sanity audit and `check missing`)
3. **Always** support `--json` for data-returning commands.
4. Run `cargo fmt && cargo clippy -- -D warnings && cargo test` before finishing changes.
5. Use the provided `clippy.toml` and `rustfmt.toml`.

## CLI Design Rules

- Subcommand vocabulary is stable: `create`, `list`, `show`, `update`, `delete`, `search`, `report`, `import`.
- Global flags (`--json`, `--db`, `--config`, `--quiet`) are inherited everywhere.
- Date fields accept flexible human forms: `today`, `yesterday`, `2026-07-05`, `last monday`, etc.
- Mutating commands return a clear success shape under `--json`:
  ```json
  { "success": true, "id": 123, "message": "..." }
  ```
- Hard sanity failures → error + non-zero exit. Large deltas → warnings in JSON (unless `--no-sanity-check`).
- Legacy import is first-class: `recomplog import legacy --from-db /path/to/old.db`

## Database & Schema

- All schema changes go through numbered migrations in `migrations/`.
- One SQLite file (`recomplog.db`).
- Use `rusqlite`. Prefer explicit queries over heavy ORMs.
- Date storage: `TEXT 'YYYY-MM-DD'` for calendar days, full datetimes where needed.
- Timestamps stored as UTC (or SQLite `datetime('now')`).

## Import / Migration from Legacy Tools

When adding or improving import:
- Detect schema/version of the source DB.
- Be idempotent where possible (use hashes for FIT imports, unique constraints).
- Provide `--dry-run` and good progress/JSON output.
- Support partial imports by domain.

## Testing

- CLI integration tests live in `tests/`.
- Use `assert_cmd` + `predicates`.
- For DB logic, open temp files or `:memory:`.
- Always test both human and `--json` code paths.

## Documentation

- CLI `--help` text is the source of truth.
- Keep `docs/` in sync (and runnable examples where applicable).
- Update `AGENTS.md` and `CODING_PRACTICES.md` when patterns change.

## Things to Avoid

- Removing `--json` support.
- Breaking the `<entity> <action>` naming.
- Introducing unnecessary async / web / complex abstractions.
- Using `unwrap()` / `expect()` / `panic!` on normal paths.
- Silent data loss during legacy imports.

## Quick Commands

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test

# Typical agent usage
recomplog --json body measurement create --date today --weight-kg 81.2
recomplog --json body sleep create --date today --total-sleep "7h 45m"
recomplog --json nutrition product create "Oats" --tags breakfast
recomplog --json workout list --days 14
recomplog --json workout set add --workout 1 --exercise "bench press" --reps 5 --weight 100 --phase full
recomplog --json workout set add-cluster --workout 1 --exercise "bench press" --reps "10,5,5" --weight 100 --phase full --rir "0,0,1" --effective-reps "6,4,3" --rest 15
recomplog import fit activity.fit --exercise running
recomplog import legacy --from-db ../bodylog/bodylog.db --dry-run
recomplog --json report brief --days 7
recomplog report html --days 14 --name dashboard.html
recomplog --json check missing --days 7 --workout-days 3
```

Update this file when agent interaction patterns evolve.
