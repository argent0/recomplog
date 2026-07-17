# recomplog CLI Surface

This document describes the unified command structure.

## Design Goals

- Group related concerns so the top level is not overwhelming.
- `workout | exercise | set` live together under one parent.
- `measurement | sleep` live under `body`.
- `product | purchase | consumption | nutrient` live under `nutrition`.
- `report` stays top-level for easy cross-domain use (as requested).

## Recommended Usage

### Training / Workouts

```bash
recomplog workout create --type Push
recomplog workout create --type Push --started-at "2026-07-10T17:00:00-03:00" --finished-at "2026-07-10T18:30:00-03:00"
recomplog workout update 1 --finished-at "2026-07-10 19:00"
recomplog workout list --days 14
recomplog workout show 42

recomplog workout exercise list --search bench
recomplog workout exercise create "incline dumbbell press" --category strength --equipment dumbbell

recomplog workout set add --workout 42 --exercise "bench press" --reps 5 --weight 100
# body_mass exercises: --weight is optional when a body measurement exists
recomplog workout set add --workout 42 --exercise "pull up" --reps 8
# Preview without writing (also on set/workout/exercise mutators)
recomplog --json workout set add --workout 42 --exercise "bench press" --reps 5 --weight 100 --dry-run
```

### Body Composition + Sleep

```bash
recomplog body measurement create --date today --weight-kg 80.5 --body-fat-pct 17.8 --json
recomplog body measurement list --days 30 --json
recomplog body measurement medians --window 7 --days 7 --json
recomplog body measurement show --date yesterday

recomplog body sleep create --date today --total-sleep "7h 45m"
recomplog body sleep list --days 14
```

### Nutrition

```bash
recomplog nutrition product create "Rolled Oats 1kg" --tags bulk,breakfast --json
recomplog nutrition product list --json
recomplog nutrition product search --name oats

recomplog nutrition purchase create --product 12 --quantity 1 --price 4.99

# Product nutrition: three unit kinds only — g (mass), ml (volume), unit (package)
# - g / ml: bulk & pourables (oil, oats, yogurt) — log the portion eaten, not the package
# - unit: discrete whole items only (protein bar, capsule, one prepared drink)
recomplog nutrition product nutrition set 12 --reference-quantity 100 --reference-unit g \
  --energy-kcal 389 --protein-g 17
recomplog nutrition product nutrition set 3 --reference-quantity 1 --reference-unit unit \
  --energy-kcal 180 --protein-g 15

# Consumption must use the same kind as the product (unit defaults to product reference)
recomplog nutrition consumption create --product 12 --quantity 80 --unit g --date today
recomplog nutrition consumption create --product 3 --quantity 1 --unit unit --date today
# Oil / fats: weigh the pour (e.g. 5–15 g), never the whole bottle
recomplog nutrition consumption create --product 16 --quantity 7 --unit g --date today
```

### Reports (top-level)

```bash
recomplog report html --days 14 --name dashboard.html
recomplog report body --days 30

# Multi-section terminal brief (focal-day consumption + workouts, then N-day lists)
recomplog report brief
recomplog report brief --days 14
recomplog report brief --date yesterday --days 7
recomplog --json report brief --days 7
recomplog --json report brief --date 2020-06-15 --days 3

# Nutrition: period totals (macros + micronutrients)
recomplog --json report nutrition summary --days 7
recomplog --json report nutrition summary --since 2026-05-01 --until 2026-05-31

# Nutrition: per-day rollup (--value filters a single macro; default macronutrients)
recomplog --json report nutrition list --days 7 --value protein
recomplog report nutrition list --value macronutrients --days 14

# Spending: total + by store always; --by product adds product breakdown
recomplog --json report nutrition spending --days 30 --by store
recomplog --json report nutrition spending --since 2026-01-01 --by product
```

`report brief` prints (human) or returns (JSON) in one shot:

1. Focal-day consumptions (default: today; set with `--date`)
2. Nutrition by day (macros, last N days ending on the focal day; default 7)
3. Measurements (last N days ending on the focal day)
4. Sleep (last N days ending on the focal day)
5. Focal-day workouts **in full detail** (same shape as `workout show`: exercises + sets)
6. Previous N days workout overview (session/volume stats + compact list)

Query `--date` / `--since` / `--until` accept flexible forms (`today`, `yesterday`, `YYYY-MM-DD`, `last monday`, …). Lookback (`--days`) is inclusive of the anchor day for nutrition/body/sleep; previous workouts use the N days *before* the anchor.

**Event time vs storage time:** log creates always record *when it happened* (event) separately from *when it was stored* (`created_at` = now). Example: log a 09:00 meal at noon with `--consumed-at 2026-07-14T09:00:00-03:00`.

**Create/update event instants** (workout `--started-at` / `--finished-at`, nutrition `--purchased-at` / `--consumed-at`; `--date` is still accepted as an alias) require **RFC3339** (any offset; stored as UTC `…Z`). Consumption at local midnight is refused unless `--allow-midnight` (discouraged). Body `--date` remains a flexible **event calendar day**.

Nutrition report date flags: `--days N` cannot be combined with `--since` / `--until`.

### Import (including legacy databases)

```bash
recomplog import legacy --from-db ~/.local/share/bodylog/bodylog.db --dry-run
recomplog import legacy --from-db ../old-nutlog.db
recomplog import legacy --from-db ../old-repslog.db --domain workout
recomplog import fit activity.fit
```

Legacy **workout** import copies the full set skeleton plus, when present in the source DB:
`activity_imports` (FIT SHA / device metadata), `activity_trackpoints`, and cardio set
fields including `heart_rate_zones` / `laps`. Dry-run reports per-table `would_copy`
counts (including provenance tables). Re-runs are idempotent via `INSERT OR IGNORE`.

### Other top-level

```bash
recomplog check --variations
# Audits body measurements, sleep, and exercise sets against configured sanity limits.
# Sets use absolute limits only (date window = workout session day).

# Completeness: missing daily logs (measurement, sleep, nutrition) over last N days
# (includes today), plus workout inactivity over last M days.
recomplog check missing --days 7 --workout-days 3
recomplog --json check missing --days 7 --workout-days 3
# End window at yesterday (do not require today's logs yet):
recomplog --json check missing --days 7 --workout-days 3 --skip-today

recomplog config generate
recomplog init
```

## Shell completion

Completions are **dynamic**: the shell registers a small function that calls
`recomplog` again under `COMPLETE=$shell`. Prefer re-sourcing on shell startup
(not a stale file on disk) so the protocol stays aligned with the binary:

```bash
# Bash
source <(COMPLETE=bash recomplog)

# Zsh
source <(COMPLETE=zsh recomplog)

# Fish
COMPLETE=fish recomplog | source
```

With this enabled, tab-complete top-level groups (`workout`, `body`, …), nested
actions, flags, fixed enums (e.g. `--phase`), and live DB values (exercise
names, product/workout/store ids). Dynamic values use the **default** DB path
during completion (global `--db` on the partial line is not consulted yet).

## Global Flags

All commands accept:
- `--json`
- `--db PATH`
- `--config PATH`
- `--quiet`

## Legacy Import Domains

- `workout` — exercises, workouts, sets (cardio scalars + zones/laps), optional `activity_imports` / `activity_trackpoints`
- `body`
- `nutrition`

The importer auto-detects based on tables present in the source database. Source columns that are missing on older schemas are skipped; missing provenance tables are no-ops.

## Notes for Agents

Always prefer `--json` when scripting or being called by LLMs.

The structure is intentionally regular: `<group> <entity> <action>` where possible.

## Advanced (parity)

```bash
# Sets
recomplog workout set add-cluster --workout 1 --exercise "bench press" \
  --reps "10,5,5" --weight 100 --phase full --rir "0,0,1" --effective-reps "6,4,3" --rest 15
recomplog workout set add-cardio --workout 1 --exercise running \
  --distance 5 --duration 1500 --avg-heart-rate 150 --max-heart-rate 175 --pace 5 --calories 400 \
  --cadence 170 --ascent 120 --descent 115 \
  --hr-zones '{"z1_seconds":60,"z2_seconds":1200,"z3_seconds":240,"z4_seconds":0,"z5_seconds":0}'
# Optional strict cardio (require zones + laps JSON)
recomplog workout set add-cardio ... --require-zones-laps --hr-zones '...' --laps '...'
# Update zones / cadence after import
recomplog workout set update 9 --hr-zones '...' --laps '...' --cadence 172 --ascent 125

# Nutrition micros + store
recomplog nutrition store create "Local Market"
recomplog nutrition product nutrition set 1 --reference-quantity 100 --reference-unit g \
  --energy-kcal 59 --protein-g 10 --micronutrient Magnesium 200 mg
# Oil / bulk fat — mass product; consumption is grams poured, not the bottle
recomplog nutrition product nutrition set 16 --reference-quantity 100 --reference-unit g \
  --energy-kcal 884 --fat-g 100
# Package product (one bar / capsule / drink = 1 unit; not for pourable oils):
recomplog nutrition product nutrition set 3 --reference-quantity 1 --reference-unit unit \
  --energy-kcal 180 --protein-g 15

# FIT
recomplog import fit activity.fit --exercise running --dry-run
recomplog import fit activity.fit --hr-zone-bounds 120,140,160,175,190
```
