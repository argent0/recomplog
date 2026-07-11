# Legacy import fixtures

## Policy

- **Do not check in real user databases.** Keep synthetic, privacy-safe data only.
- Root `.gitignore` ignores `*.db` / `*.sqlite`, so binary SQLite fixtures are not the default.
- Prefer **in-test builders** that create minimal legacy-shaped DBs with `rusqlite` at runtime.

## Canonical builders

See `tests/import_legacy.rs`:

| Helper | Domain | Covers |
|--------|--------|--------|
| `build_bodylog_min` | body | measurements + sleep |
| `build_nutlog_min` | nutrition | product, nutrition, purchase, consumption |
| `build_repslog_min` | workout | exercises, workout, sets, activity_imports, trackpoints |

Each builder matches the tables/`SELECT`s used by `src/commands/import.rs` (`copy_body`, `copy_nutrition`, `copy_workout`).

## FIT fixture (separate)

Canonical path (not under this directory):

```text
tests/fixtures/Zepp20260710164935.fit
```

Used by `tests/import_fit.rs` and the unit test in `src/fit/parse.rs`. Do not place a second copy at the repo root.

## Extending

1. Copy an existing `build_*_min` helper.
2. Keep only the columns the importer reads.
3. Assert round-trip counts and a re-import idempotency check.
4. Keep tests offline (no network, no private dumps).
