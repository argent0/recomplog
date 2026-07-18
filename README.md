# recomplog

Unified local CLI for body recomposition tracking.

Single-user, local-first, and agent-friendly. One SQLite database for:

- **Training** — workouts, exercises, sets (strength, cluster, cardio), FIT import
- **Body** — measurements, sleep, profile (height / DOB / HR zones)
- **Nutrition** — products, purchases, consumption, stores, micronutrients
- **Reports** — body/sleep/nutrition summaries, combined JSON, self-contained HTML dashboard

Successor to `repslog`, `bodylog`, `nutlog`, and `bodydashboard`.

## Status

The grouped CLI is in place for body, nutrition, workout (including stats), reports, config/check, legacy DB import, and **FIT import** (Garmin/Zepp-style activities, idempotent by file hash, optional HR zones).

`--json` is supported on data-returning commands. CLI `--help` is the source of truth for flags; `docs/cli.md` describes the full surface.

## Install

**From source** (Rust toolchain required):

```bash
cargo install --path .
# or development builds:
cargo build --release
# binary: target/release/recomplog
```

**Arch Linux** (VCS package in-tree):

```bash
# from a clean checkout
makepkg -si
```

See `PKGBUILD` for depends/makedepends and install layout.

## Shell completion

```bash
recomplog config bash-completion   # bash: append to ~/.bashrc (idempotent)
source ~/.bashrc
```

Dynamic completions re-invoke the binary at tab time. Other shells:

```bash
# Zsh
echo 'source <(COMPLETE=zsh recomplog)' >> ~/.zshrc

# Fish
echo 'COMPLETE=fish recomplog | source' >> ~/.config/fish/completions/recomplog.fish
```

## Data locations

| What   | Default path                                      |
|--------|---------------------------------------------------|
| DB     | `~/.local/share/recomplog/recomplog.db`           |
| Config | `~/.config/recomplog/config.toml`                 |

Override with global `--db PATH` and `--config PATH`.

## Global flags

Available on all commands:

| Flag         | Purpose                                      |
|--------------|----------------------------------------------|
| `--json`     | Structured JSON (preferred for agents/scripts) |
| `--db PATH`  | Override SQLite path                         |
| `--config PATH` | Override config path                      |
| `--quiet`    | Minimal human output                         |

Dates accept flexible forms: `today`, `yesterday`, `2026-07-05`, `last monday`, etc.

## Quick start

```bash
# Body
recomplog body measurement create --date today --weight-kg 80.5 --json
recomplog body measurement list --days 14
recomplog body measurement medians --window 7 --days 7
recomplog body sleep create --date today --total-sleep "7h 45m"
recomplog body profile set --height-cm 178

# Training
recomplog workout create --type Push
recomplog workout exercise list --search bench
recomplog workout set add --workout 1 --exercise "bench press" --reps 5 --weight 100 --phase full
recomplog workout list --days 14
recomplog workout stats volume --days 30
recomplog workout show 1

# Nutrition
recomplog nutrition product create "Oats" --tags breakfast
recomplog nutrition product list --json
recomplog nutrition consumption create --product 1 --quantity 0.8 --date today

# Reports
recomplog report brief --days 7
recomplog report html --days 14 --name dashboard.html
recomplog --json report nutrition summary --days 7
recomplog report body --days 30

# Import
recomplog import fit activity.fit --exercise running --dry-run
recomplog import legacy --from-db ../bodylog/bodylog.db --dry-run
recomplog import legacy --from-db ../nutlog/nutlog.db

# Sanity / config / database
recomplog db check --variations
recomplog db check missing --days 7 --workout-days 3
recomplog db backup
recomplog db backup --to ~/backups/
recomplog config generate
```

## Command groups

```
recomplog workout    # sessions, exercises, sets, stats
recomplog body       # measurement, sleep, profile
recomplog nutrition  # product, purchase, consumption, micronutrient, store, tags
recomplog report     # brief, nutrition, body, sleep, summary, html
recomplog import     # fit | legacy
recomplog config     # show | generate | path
recomplog db         # backup | migrate | check (sanity audit / missing logs)
recomplog init       # one-time setup helpers
```

Shape is generally `<group> <entity> <action>`. Full examples (clusters, cardio sets, nutrition micros, spending reports): **`docs/cli.md`**.

## Develop

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

Integration tests live under `tests/`. Conventions for agents and contributors: **`AGENTS.md`**, **`CODING_PRACTICES.md`**.

License: MIT (see `Cargo.toml`).
