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
recomplog report nutrition list --days 7
```

### Import (including legacy databases)

```bash
recomplog import legacy --from-db ~/.local/share/bodylog/bodylog.db --dry-run
recomplog import legacy --from-db ../old-nutlog.db
recomplog import fit activity.fit
```

### Other top-level

```bash
recomplog check --variations
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

- `workout`
- `body`
- `nutrition`

The importer auto-detects based on tables present in the source database.

## Notes for Agents

Always prefer `--json` when scripting or being called by LLMs.

The structure is intentionally regular: `<group> <entity> <action>` where possible.
