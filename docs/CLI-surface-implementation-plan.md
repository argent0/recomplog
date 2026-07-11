# Implementation Plan: New Grouped CLI Surface for recomplog

**Status:** Major phases implemented (body, nutrition, workout core, reports/HTML, legacy import). FIT import still stubbed; some advanced set modalities (cluster) and full micronutrient UI remain thinner than source tools.

**Goal:** Deliver a clean, predictable, agent-friendly CLI with logical grouping while preserving (and improving upon) the behavior and power of the original four tools.

## 1. Current State (as of initial commit)

- Top-level commands restructured:
  - `recomplog workout ...`
    - `create`, `list`, `show`
    - `exercise <action>`
    - `set <action>`
  - `recomplog body ...`
    - `measurement <action>` (create/list/show/update/delete)
    - `sleep <action>`
  - `recomplog nutrition ...`
    - `product`, `purchase`, `consumption`, `nutrient`
  - `recomplog report ...` (intentionally top-level)
  - `import`, `config`, `check`, `init`, `migrate` (top-level utilities)
- Basic working implementations exist for:
  - Several `body measurement` and `nutrition product` flows
  - Basic `workout` + `workout exercise`
  - Real data copying in `import legacy` for `body` and `nutrition`
- Schema is unified in one DB.
- `--json`, global flags, XDG paths, and flexible date parsing are present.

**Gaps:** Most subcommands are stubs or minimal. No full parity with original tools yet. No rich output tables in many places. No sanity checking. No full cross-domain reports.

## 2. Target CLI Surface (Reference)

See `docs/cli.md` for user-facing examples.

Key principles:
- Related commands live together (`workout exercise`, `body measurement`, `nutrition product`).
- `report` remains easily accessible at the top level.
- Every data-returning command supports `--json`.
- Consistent verb vocabulary: `create`, `list`, `show`, `update`, `delete`, `search`, `add`.
- Help text is the primary documentation.

## 3. High-Level Phases

### Phase 0: Foundations (mostly done)
- [x] Grouped clap structure
- [x] Basic dispatch
- [x] DB + migrations
- [x] Initial legacy import (body + nutrition)
- [x] Common `Success` envelope + JSON helpers

### Phase 1: Core Data Layer & Shared Utilities
- Create a proper `src/repository.rs` (or domain-specific repositories) using rusqlite.
- Port / implement flexible date parsing (full version from bodylog/nutlog).
- Implement common timestamp types (`TimestampInfo`).
- Bring over `Success` variants with `warnings` (for sanity) and `deleted_id`.
- Central error type (`RecomplogError`) with good context.
- Shared output helpers:
  - `print_table` (repslog-style header-only or comfy-table)
  - `quiet_print`
  - Consistent JSON error shape

### Phase 2: Body Domain (measurement + sleep)
Priority: high (simple tables, used by reports).

- `body measurement`
  - `create` (with sanity checks from bodylog)
  - `list` (with --days / --since / --until, newest first)
  - `show` (by id or --date)
  - `update` (partial)
  - `delete`
- `body sleep`
  - Full create (all fields: bedtime, stages, vitals, notes, etc.)
  - list / show / update / delete
- `body` level config integration (height, DOB via `user_profile`).
- Port sanity logic (`src/sanity.rs` + config).

Deliverables:
- Full parity with original `bodylog measurement` and `sleep`.
- `--no-sanity-check` support.
- `body check` (or top-level `check`) works for body data.

### Phase 3: Nutrition Domain
- `nutrition product`
  - create, list, search (fuzzy), show, rename, delete, tag add/remove
  - `nutrition set` (full micronutrient support)
- `nutrition purchase` (with price in cents, store)
- `nutrition consumption`
- `nutrition nutrient` (CRUD + search)
- Tags (product-tag, store-tag), stores
- Proper fuzzy search (token-aware Jaro-Winkler from nutlog)

### Phase 4: Workout / Training Domain (most complex)
This is the richest domain from repslog.

Under `recomplog workout`:

- `create` / `list` / `show` / delete
- `exercise` subcommand (list, create, search, update) — names must stay lowercase/singular
- `set` subcommands:
  - `add` (strength)
  - `add-cardio`
  - `add-cluster`
  - `add-unilateral`
  - update, delete
- Support for:
  - load_type, phase, side, rir, effective_reps, external_load
  - HR zones, laps, trackpoints, cadence, ascent, etc.
  - FIT provenance (`activity_imports`)

Also support the old top-level feel via good help and aliases if needed.

### Phase 5: Reports & Dashboard (cross-domain)
- `report nutrition ...` (daily + period totals, micronutrients, spending)
- `report body ...` (individual metrics + summary)
- `report sleep ...`
- `report summary` (combined recomposition view)
- `report html` — reimplement using direct DB queries + the original Chart.js template from bodydashboard.
  - Pull measurements, sleep, nutrition in one pass.
  - Compute body metrics (fat mass, muscle mass, etc.).
  - Generate self-contained `index.html`.

Goal: `bodydashboard` functionality becomes native and much more reliable.

### Phase 6: Import & Migration Polish
- Complete `import legacy`:
  - Workout domain (exercises → workouts → workout_exercises → sets, handling FK order and activity_imports)
  - Idempotency / conflict handling
  - Good progress output and `--json`
- `import fit` — port the entire `repslog fit/` parser + mapping logic.
- Add `import` subcommands for other formats if useful.

### Phase 7: Config, Sanity, Check
- Unified `~/.config/recomplog/config.toml`
- Sections: `[sanity.measurements]`, `[sanity.sleep]`, `[sanity.workout]`
- `config generate`, `config path`, `config show`
- Full `check` (and `check --variations`)
- Workout sanity (repslog limits for weight, distance, duration, HR, etc.)

### Phase 8: Polish, Testing, Documentation
- Human output tables everywhere (consistent style).
- Full `--json` contract tests.
- Integration tests using `assert_cmd` (one test DB per domain group).
- Update all `docs/`.
- Port testable examples + `verify_examples.sh` style verification.
- Update `AGENTS.md` with new command patterns.
- AUR/PKGBUILD adjustments (single binary now).
- Migration guide for users of the old separate tools.

## 4. Implementation Guidelines

### Command Structure in Code
Keep the hierarchy in `src/cli.rs` using nested `Subcommand` enums.

Example pattern (already partially done):

```rust
Commands::Workout { action: WorkoutAction }
Commands::Body { action: BodyAction }
```

Inside handlers (`src/commands.rs` or better: `src/commands/workout.rs`, etc.):

- One handler function per top-level group.
- Use a small `Repository` or direct connection for now.
- Always branch on `json` vs human early.

### Recommended Module Layout (future refactor)
```
src/
  main.rs
  cli.rs
  db.rs
  config.rs
  error.rs
  utils.rs
  models/               # shared + domain models
  repository.rs         # or per-domain
  commands/
    mod.rs
    workout.rs
    body.rs
    nutrition.rs
    report.rs
    import.rs
    config.rs
  sanity.rs
  fit/                  # ported from repslog
```

Start by expanding inside `commands.rs`, then split when it gets large.

### Output Consistency
- Human: Use the "header only" comfy-table preset from nutlog or the one from repslog.
- JSON: Always pretty-printed. Use the `Success` envelope for mutations.
- Quiet mode: Suppress decorative text.

### Date Handling
Centralize in `utils.rs`:
- `parse_flexible_date`
- `parse_flexible_datetime`
- `format_local`

Support the full set from the original tools.

### Testing Strategy
- Unit tests for date parsing, sanity, report calculations.
- CLI integration tests per group:
  ```rust
  // tests/body_measurement.rs
  // tests/nutrition_product.rs
  // tests/workout.rs
  ```
- Use `recomplog::db::open_db` with temp files or in-memory where possible.
- Always test both `--json` and human paths.

### Legacy Import Strategy
- Detect domain by table presence (already done).
- Copy in FK-safe order.
- For workout: copy exercises first, then workouts, then workout_exercises, then sets + imports.
- Record provenance where possible (`imported_from` notes or new table).

## 5. Risks & Open Questions

- Workout domain has many columns and JSON fields (`heart_rate_zones`, `laps`). Need good serde helpers.
- Some original commands used `workout-exercise` as a top-level command. We can support `recomplog workout exercise ...` and optionally a convenience alias later.
- Should we provide shims (e.g. `recomplog measurement` as alias to `recomplog body measurement`)? Probably not — document the new structure instead.
- Config unification: bodylog's rich sanity vs repslog's simpler one. Design a superset.

## 6. Suggested Order of Work (for agents / contributors)

1. Phase 1 (shared utils + repository)
2. Phase 2 (body domain) — quick wins + used by reports
3. Phase 3 (nutrition domain)
4. Phase 5 (reports + html) — high value
5. Phase 4 (workout) — largest effort
6. Phase 6 + 7 (import + config)
7. Phase 8 (tests + docs)

## 7. Definition of Done

- All major commands from the three source tools work under the new grouped surface.
- `recomplog report html` produces a useful dashboard pulling from all domains.
- `import legacy` can migrate a full user's data from the three old DBs.
- `cargo test` + docs verification pass.
- `--help` is excellent.
- New users + LLM agents can discover and use the tool effectively.

---

Update this plan as decisions are made during implementation.
