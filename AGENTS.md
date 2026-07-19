# AGENTS.md — Guidelines for LLM Agents Working on recomplog

`recomplog` is the unified successor to `repslog`, `bodylog`, `nutlog`, and `bodydashboard`.

It is a **single-user, local-first, LLM-agent-first** CLI tool for body recomposition tracking: strength & cardio workouts, body measurements, sleep, nutrition logging, and HTML reports.

## Core Philosophy (inherited and extended)

- Simplicity first — boring, maintainable, explicit.
- Agent-friendly by design — consistent `entity action`, excellent `--json`, predictable behavior.
- One database for everything (no more fragile subprocess + JSON glue between tools).
- Preserve the spirit of the original four tools.
- **Append only — no exceptions. Never, nowhere.**
  - Event history (workouts, sets, consumptions, purchases, measurements, sleep, imports, trackpoints, …) **grows only by insertion**. There is no carve-out, domain, import path, migration, “convenience upsert”, day-uniqueness, or bulk “fix” that may rewrite settled event rows. If a design needs to mutate past events, the design is wrong — append a new fact (or supersede/void via a later row when that model exists), do not overwrite the old one.
  - Logging path is always `create` / `add` / import-insert. Same calendar day may have many measurement or sleep samples; `create` always inserts. No `UNIQUE(event day)`, no create-fails-if-exists, no second-write-as-update.
  - Catalog and config (products, exercises, tags, profile, micronutrients, nutrition facts) may be updated or merged; that is catalog, not event history. Product merge soft-retires sources as aliases (`merged_into_id` / `retired_at`) and must **not** rewrite consumption/purchase `product_id`.
  - Imports append and stay idempotent (`INSERT OR IGNORE` / hash skip). Never replace domain history as a side effect. Reports and checks **read** the log as stored; day-series aggregation (e.g. last-by-`created_at` per date) is read-side only and never deletes prior samples.
  - Event time and storage time stay independent: appending a late log never backdates `created_at` (see time model below).
  - Existing `update` / `delete` on log rows are **legacy correction debt**, not a model for new work. New features must not depend on them. Prefer append (or future supersede/tombstone) over rewrite.
  - Event `update` is classified: **lifecycle** (fill nulls, e.g. first `finished_at`) vs **correction** (overwrite settled values). Corrections require `--reason` and write audit `kind: correct` with field old/new; inspect via `… audit <id>`.
- **Quality data produces quality reports. Quality reports are actionable reports.**
  - Logging, imports, sanity checks, and `db check` exist so the data is trustworthy.
  - Reports (`report brief`, domain summaries, HTML) exist so the user (or agent) can act — not just stare at numbers.
  - Prefer features that raise data quality or make reports more decision-ready over vanity metrics.

## How to Work as an Agent

1. **Start here**: Read `AGENTS.md`, `CODING_PRACTICES.md`, `README.md`, and relevant `docs/`.
2. Use the standard command shape: `recomplog <group> <entity> <action> ...`
   - Training: `workout create|list|show|delete`, `workout exercise ...`, `workout set add|add-cardio|delete`
   - Body: `body measurement ...`, `body sleep ...`, `body profile ...`
   - Nutrition: `nutrition product|purchase|consumption|micronutrient|infoods ...`
   - Cross-cutting: `report` (including `report brief`), `import`, `config`, `db` (`backup`, `migrate`, `check` / `check missing`)
3. **Always** support `--json` for data-returning commands.
4. Run `cargo fmt && cargo clippy -- -D warnings && cargo test` before finishing changes.
5. Use the provided `clippy.toml` and `rustfmt.toml`.

## Time model (event vs storage)

**Never conflate “when it happened” with “when it was stored.”**

Example: at 12:00 the user logs “I ate at 09:00” → `consumed_at = 09:00`, `created_at = 12:00`.
Reports, day buckets, and `db check missing` use **event** time only.

| Kind | Meaning | Examples | Who sets it |
|------|---------|----------|-------------|
| **Event** | When the user says it occurred | `started_at` / `finished_at`, `consumed_at`, `purchased_at`, `measurements.date`, `sleep.date`, `recorded_at` | User / device / import payload |
| **Storage** | When recomplog wrote or last updated the row | `created_at`, `updated_at`, `imported_at` | Always app `now_utc()`; **never** user-editable |

- Log creates always set **both** clocks independently.
- Catalog entities (`product`, `exercise`, tags, …) only have storage time.
- `exercise_sets.created_at` is **log** time; the session day comes from the parent workout.
- Create/update **event instants** accept **RFC3339 only** (`--started-at`, `--consumed-at`, `--purchased-at`; `--date` is an alias for nutrition event instants).
- Event **calendar days** and **query** flags stay flexible (`today`, `yesterday`, `2026-07-05`, …).
- Nutrition consumption refuses local midnight on the **event** instant unless `--allow-midnight`.

## CLI Design Rules

- Subcommand vocabulary is stable: `create`, `list`, `show`, `update`, `delete`, `search`, `report`, `import`.
- Global flags (`--json`, `--db`, `--config`, `--quiet`) are inherited everywhere.
- Mutating log creates under `--json` include storage + event keys, e.g.:
  ```json
  { "success": true, "id": 123, "created_at": "…Z", "consumed_at": "…Z", "message": "..." }
  ```
  Measurement/sleep keep calendar `date` as the event day and still include `created_at`.
- Hard sanity failures → error + non-zero exit. Large deltas → warnings in JSON (unless `--no-sanity-check`).
- Legacy import is first-class: `recomplog import legacy --from-db /path/to/old.db`

## Database & Schema

- All schema changes go through numbered migrations in `migrations/`.
- One SQLite file (`recomplog.db`).
- Use `rusqlite`. Prefer explicit queries over heavy ORMs.
- Date storage: `TEXT 'YYYY-MM-DD'` for calendar days (`measurements.date`, `sleep.date`, DOB).
- Instants (points in time) stored as UTC RFC3339 with `Z` only: `YYYY-MM-DDTHH:MM:SSZ`
  (`started_at`, `finished_at`, `recorded_at`, `created_at`/`updated_at`/`imported_at`,
  `purchased_at`, `consumed_at`). Legacy naive values are Buenos Aires (UTC−3).

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
- Features that treat bulk rewrite of historical event rows as the normal path (violates append-only).

## Quick Commands

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test

# Typical agent usage
recomplog --json body measurement create --date today --weight-kg 81.2
recomplog --json body measurement correct --id 3 --weight-kg 80.5 --reason "scale typo"
recomplog --json body measurement medians --window 7 --days 7
recomplog --json body sleep create --date today --total-sleep "7h 45m"
recomplog --json body sleep correct --id 5 --total-sleep "7h 50m" --reason "watch resync"
recomplog --json nutrition product create "Oats" --tags breakfast
# Merge duplicates as aliases: retire sources onto --into (event product_ids stay; tags/nutrition gaps copy)
recomplog --json nutrition product merge --into 14 61
recomplog --json nutrition product merge --into 14 61 --name "Morixe Instant Oats" --dry-run
recomplog --json nutrition product show 61
# Nutrition units: g (mass), ml (volume), unit (package) — consumption must match product kind.
# unit = whole discrete item (bar, capsule); pourables (oil, bulk) use g and log the portion only.
recomplog --json nutrition product nutrition set 3 --reference-quantity 1 --reference-unit unit --energy-kcal 180 --protein-g 15 --carbohydrates-g 18 --fat-g 7 --fiber-g 0 --sugars-g 2
# Event time ≠ storage time (log meal that happened earlier today)
recomplog --json nutrition consumption create --product 3 --quantity 1 --unit unit --consumed-at 2026-07-14T13:45:00-03:00
recomplog --json nutrition consumption create --product 12 --quantity 80 --unit g --consumed-at 2026-07-14T08:30:00-03:00
# Wrong meal: append-only correct (new row supersedes old; old soft-deleted)
recomplog --json nutrition consumption correct 88 --quantity 90 --unit g --reason "weighed again"
recomplog --json nutrition purchase create --product 3 --quantity 2 --purchased-at 2026-07-14T18:00:00-03:00
recomplog --json nutrition purchase correct 12 --quantity 3 --reason "bought three"

recomplog --json workout create --type Push --started-at 2026-07-14T17:00:00-03:00
# Lifecycle fill (no --reason): first finished_at on an open session
recomplog --json workout update 1 --finished-at 2026-07-14T18:30:00-03:00
# Correction overwrites need --reason; audit shows kind correct + fields
recomplog --json workout update 1 --finished-at 2026-07-14T19:00:00-03:00 --reason "clock typo"
recomplog --json workout list --days 14
# Soft-delete keeps history; --purge --force hard-removes CASCADE trees
recomplog --json workout delete 1 --reason "abandoned"
recomplog --json workout audit 1
recomplog --json workout exercise audit 3
recomplog --json workout set audit 1
recomplog --json nutrition consumption delete 88 --reason "duplicate"
recomplog --json nutrition consumption audit 88
recomplog --json nutrition product audit 14
recomplog --json body measurement update --id 3 --weight-kg 80.5 --reason "scale typo"
recomplog --json body measurement audit --id 3
recomplog --json body sleep audit --date yesterday
recomplog --json workout set add --workout 1 --exercise "bench press" --reps 5 --weight 100 --phase full
recomplog --json workout set update 1 --reps 6 --reason "miscount"
# body_mass: --weight optional when a body measurement exists
recomplog --json workout set add --workout 1 --exercise "pull up" --reps 8
recomplog --json workout set add-cluster --workout 1 --exercise "bench press" --reps "10,5,5" --weight 100 --phase full --rir "0,0,1" --effective-reps "6,4,3" --rest 15
recomplog import fit activity.fit --exercise running
recomplog import legacy --from-db ../bodylog/bodylog.db --dry-run
recomplog --json report brief --days 7
recomplog --json report brief --date yesterday --days 7
recomplog report html --days 14 --name dashboard.html
recomplog db backup
recomplog --json db backup --to ~/backups/
recomplog --json db check missing --days 7 --workout-days 3
recomplog --json db check missing --days 7 --workout-days 3 --skip-today
# Micronutrients: prefer INFOODS tags for classics; product set auto-links exact INFOODS names
recomplog --json nutrition infoods search "iron"
recomplog --json nutrition micronutrient create Iron --unit mg --infoods FE
recomplog --json db check   # fails if any micronutrient lacks infoods_tag
```

Update this file when agent interaction patterns evolve.
