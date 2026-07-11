# recomplog

Unified local CLI for body recomposition tracking.

Combines:
- Workout / reps / sets / FIT tracking (from repslog)
- Body measurements + sleep (from bodylog)
- Nutrition / food logging + reports (from nutlog)
- HTML dashboard reports (from bodydashboard)

All data in a single SQLite database. Excellent support for both humans and LLM agents via `--json`.

## Status

Early merge in progress. Core functionality from the four tools is being unified.

See `AGENTS.md` and `CODING_PRACTICES.md` for contribution guidelines.

## Quick Start

```bash
# Body
recomplog body measurement create --date today --weight-kg 80.5 --json
recomplog body measurement list --days 14

# Training
recomplog workout create --type Push
recomplog workout exercise list
recomplog workout set add --workout 1 --exercise "bench press" ...

# Nutrition
recomplog nutrition product create "Oats" --tags breakfast
recomplog nutrition product list --json

# Reports (top level)
recomplog report html --days 14

# Migrate from old tools
recomplog import legacy --from-db ../bodylog/bodylog.db
recomplog import legacy --from-db ../nutlog/nutlog.db
```

See `docs/cli.md` for the full grouped command surface.
```

Data locations:
- DB: `~/.local/share/recomplog/recomplog.db`
- Config: `~/.config/recomplog/config.toml`
