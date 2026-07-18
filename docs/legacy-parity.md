# Legacy tool → recomplog command map

| Legacy | recomplog | Status |
|--------|-----------|--------|
| bodylog measurement * | `body measurement *` | done |
| bodylog sleep * | `body sleep *` | done |
| bodylog check | `db check` | done |
| bodylog config set/show | `body profile set/show` | done |
| nutlog product * | `nutrition product *` | done |
| nutlog product nutrition set | `nutrition product nutrition set` | done |
| nutlog product-tag * | `nutrition product-tag *` | done |
| nutlog store * | `nutrition store *` | done |
| nutlog store-tag * | `nutrition store-tag *` | done |
| nutlog purchase/consumption/nutrient | `nutrition purchase/consumption/nutrient` | done |
| nutlog report | `report nutrition *` | done |
| repslog workout * | `workout *` | done |
| repslog exercise * | `workout exercise *` | done |
| repslog set add/add-cardio/add-cluster/add-unilateral | `workout set …` | done |
| repslog set update/move/list/quick/delete | `workout set …` | done |
| repslog import fit | `import fit` | done |
| repslog stats | `workout stats` | done |
| bodydashboard HTML | `report html` | done |
| import legacy DBs | `import legacy --from-db` | done |

No top-level aliases for old names (by design).

## Cardio zones / laps strictness

repslog often required structured `--hr-zones` and `--laps` for cardio sets.
recomplog accepts them as **optional** on `workout set add-cardio` (and FIT import
fills them when present). Opt into strict validation with:

```bash
recomplog workout set add-cardio ... --require-zones-laps --hr-zones '...' --laps '...'
```

Manual cadence/elevation (`--cadence`, `--ascent`, `--descent`) map to
`avg_cadence_spm` / `total_ascent_m` / `total_descent_m` on add and update
(FIT import already populated these).

## Dry-run on mutations

`workout create|update|delete`, `workout exercise create|update`, and all
`workout set` mutators accept `--dry-run`: resolve IDs, run sanity validation,
return `{ "success": true, "dry_run": true, "would": { ... } }` under `--json`,
and perform **no** DB writes (including auto-creating `workout_exercises`).
Validation failures still exit non-zero.
