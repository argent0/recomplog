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
recomplog workout list --days 14
recomplog workout show 42

recomplog workout exercise list --search bench
recomplog workout exercise create "incline dumbbell press" --category strength --equipment dumbbell

recomplog workout set add --workout 42 --exercise "bench press" --reps 5 --weight 100
```

### Body Composition + Sleep

```bash
recomplog body measurement create --date today --weight-kg 80.5 --body-fat-pct 17.8 --json
recomplog body measurement list --days 30 --json
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
recomplog nutrition consumption create --product 12 --quantity 0.8 --date today
```

### Reports (top-level)

```bash
recomplog report html --days 14 --name dashboard.html
recomplog report body --days 30

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
recomplog config generate
recomplog init
```

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
  --distance 5 --duration 1500 --avg-heart-rate 150 --max-heart-rate 175 --pace 5 --calories 400

# Nutrition micros + store
recomplog nutrition store create "Local Market"
recomplog nutrition product nutrition set 1 --reference-quantity 100 --reference-unit g \
  --energy-kcal 59 --protein-g 10 --micronutrient Magnesium 200 mg

# FIT
recomplog import fit activity.fit --exercise running --dry-run
recomplog import fit activity.fit --hr-zone-bounds 120,140,160,175,190
```
