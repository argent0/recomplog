# recomplog — Remaining Legacy Parity Gaps

**Date:** 2026-07-10  
**Scope:** Gaps vs full behavioral parity with `repslog`, `nutlog`, `bodylog`, and `bodydashboard` under the grouped CLI  
**Baseline:** Grouped CLI implemented; FIT E2E with `tests/fixtures/Zepp20260710164935.fit` green; body/nutrition CRUD and core set modalities largely present  

This report lists what is **still missing or thinner** than the source tools. It is not a changelog of completed work.

---

## Executive summary

| Area | Rough parity | Main holes |
|------|--------------|------------|
| Body (measurement / sleep / profile) | High | Small polish only |
| `check` (body + sleep + sets) | High | Workout set audit done (2026-07-10); variations remain body-only |
| Nutrition CRUD + tags/stores | High | Nutrient seed; edge polish |
| Nutrition reports | High | Closed 2026-07-10 (summary, micros, spending-by, value filters) |
| Workout logging (sets) | High | dry-run, manual cadence/elevation, finished_at, strict cardio |
| Workout analysis | Medium–low | PRs, history, progression, track_metrics on show |
| FIT import | High (E2E) | Zone defaults; profile-path less tested |
| Legacy import | High | Provenance (trackpoints / activity_imports / zones-laps) closed 2026-07-10 |
| HTML dashboard | High | Closed 2026-07-10 (regression, stages, macros, training cards) |
| Config | Medium–high | Nested `[sanity.measurements]` layout / docs |

**Bottom line:** CRUD, FIT import, HTML dashboard depth, and legacy workout provenance migration are largely done. Remaining gaps are mainly **analysis** (track metrics on show), and **CLI polish**.

---

## 1. High impact

### 1.1 `check` does not audit workouts — **Closed 2026-07-10**

**Was:** `recomplog check` scanned measurements and sleep only.

**Now:** Also scans `exercise_sets` against `[sanity.workout]` (absolute only; date window = workout session day). See `reports/gaps/01-check-workout-audit.md`.

---

### 1.2 Track metrics stored but not computed on read

**Current:** FIT import populates `activity_trackpoints` (E2E: ~2809 points for Zepp fixture).

**Missing (repslog `track_metrics.rs`):**

- Moving vs stopped time  
- Avg/max speed from samples  
- Elevation profile from GPS  
- Zone time recomputed from samples + bounds  
- Attachment of derived metrics to `workout show` (and optionally reports)

Data lands in SQLite; the rich workout-view analysis path is not wired.

---

### 1.3 Nutrition reports — **Closed 2026-07-10**

**Was:** list as per-consumption dump; spending flat total only; no summary/micros/`--value`/`--by`.

**Now (nutlog parity):**

- `report nutrition summary` — full macros + micronutrient totals  
- `report nutrition list` — per-day rollup with `--value` (macronutrients default)  
- `report nutrition spending` — `by_store` always; `--by product` adds product breakdown; `--since`/`--until`/`--days`  
- Strict `--days` vs `--since`/`--until` (clap conflicts)  

See `reports/gaps/03-nutrition-reports.md`. (`--by month` remains out of scope / nutlog future.)

---

### 1.4 HTML dashboard thinner than bodydashboard — **Closed 2026-07-10**

**Was:** Chart.js page with weight, body fat, sleep minutes, kcal/protein; fat/lean latest cards only.

**Now:** Fat/lean time series, sleep stages + quality, full macros (protein/carbs/fat/fiber/sugars), weight/BF regression trend labels + always-present JSON `weight_trend`/`body_fat_trend`, training volume/session cards (recomplog-additive; bodydashboard has no training UI). Layout/CSS fidelity remains a non-goal. See `reports/gaps/06-html-dashboard-depth.md` and `reports/plans/06-html-dashboard-depth.md`.

---

### 1.5 Workout stats — single rollup only

**Current:** `workout stats --days N` → volume by exercise (sets, reps, kg·reps).

**Missing vs repslog `stats`:**

- Personal records (`stats prs`)  
- Volume with period string + per-exercise filter  
- Session summary (frequency, count)  
- Load progression over time for an exercise  
- Per-set history across workouts  

Agents that depended on repslog stats need rework or lose capability.

---

## 2. Medium impact

### 2.1 Set / workout CLI flags vs repslog

| Capability | Status |
|------------|--------|
| add / add-cardio / add-cluster / add-unilateral / update / move / list / quick / delete | Present |
| `--dry-run` on mutating set/workout/exercise commands | **Missing** |
| Manual `--cadence`, `--ascent`, `--descent` on set add | **Missing** (FIT path only) |
| `set update` of `hr-zones` / `laps` JSON | **Missing** |
| `add-cardio` zones/laps required (repslog strict) | **Looser** (optional here) |
| Workout `finished_at` | Schema exists; **no CLI** |

---

### 2.2 Legacy import incomplete for workout provenance — **Closed 2026-07-10**

**Was:** workout skeleton only; dropped trackpoints, `activity_imports`, cardio set fields (zones/laps).

**Now:** `copy_workout` copies preferred set columns by source∩target intersection (cardio scalars, `heart_rate_zones`, `laps`), then `activity_imports` and `activity_trackpoints` (orphan parents skipped). Dry-run reports `would_copy` counts including provenance tables. See `reports/gaps/04-legacy-import-provenance.md` and `tests/import_legacy.rs`.

---

### 2.3 Config layout vs plan

**Actual generated shape:**

```toml
[sanity]
# measurement fields flat
[sanity.sleep]
[sanity.workout]
```

**Plan preferred:** explicit `[sanity.measurements]` plus sleep/workout.

Flat layout is backward compatible; gaps:

- Docs may describe nested measurements that are not the default  
- No rewrite/migration helper to preferred shape  
- Generated TOML does not document dual layout  

---

### 2.4 Init seeding incomplete

| Seed | Status |
|------|--------|
| Default exercises | Yes (`init`) |
| Default nutrients (nutlog-style) | **No** |
| Common product tags | **No** |

---

### 2.5 Human output consistency

Some commands use header-only comfy-table; others use ad-hoc lines or raw-ish JSON for humans. Quiet mode is uneven. Does not block data correctness; hurts agent/human UX consistency.

---

## 3. Lower impact / intentional non-goals

These are **not** treated as parity bugs:

| Item | Reason |
|------|--------|
| Top-level aliases (`measurement` → `body measurement`) | Explicit non-goal |
| Async / sqlx | recomplog stays sync + rusqlite |
| Pixel-perfect bodydashboard CSS | Functional charts + domains sufficient per plan |
| Byte-identical JSON vs old tools | recomplog owns contracts |
| Interactive delete confirms | Agent-first CLI; often omitted |

---

## 4. Testing and process gaps — **Closed 2026-07-10** (gap 08)

| Gap | Detail |
|-----|--------|
| Commit hygiene | Small commits aligned with gap IDs; working tree clean on close-out |
| FIT fixture path | Canonical only: `tests/fixtures/Zepp20260710164935.fit` (parse unit test + E2E) |
| Legacy-import E2E | `tests/import_legacy.rs` — body, nutrition, workout (synthetic in-test DBs) |
| HTML tests | `tests/report_html.rs` (fat/lean, stages, macros, trends, training) |
| `check` + workout | `tests/check_workout.rs` |
| FIT profile zones | `import_fit_profile_hr_zones` when device zones null |

**FIT E2E:** `tests/import_fit.rs` (distance, duration, HR, trackpoints, dedup, `--force`, `--hr-zone-bounds`, profile zones).

---

## 5. Recommended close-out order

1. ~~Extend **`check`** to scan `exercise_sets` with `sanity.workout`.~~ **Done.**  
2. Port **`track_metrics`** into `workout show` (and optionally HTML/report).  
3. ~~Flesh out **nutrition reports** (summary, micros, spending-by, value filters).~~ **Done.**  
4. ~~Complete **legacy import** for trackpoints, `activity_imports`, zones/laps.~~ **Done.**
5. ~~Expand **stats** (`prs` / `history` / `weight` progression / summary).~~ **Done** (gap 05).  
6. ~~Deepen **HTML** (regression, sleep stages, remaining macros).~~ **Done** (gap 06).  
7. CLI polish: **`--dry-run`**, set update for JSON fields, manual cadence/ascent, `finished_at`.  
8. ~~Housekeeping: commit, single FIT fixture path, more integration tests.~~ **Done** (gap 08).

---

## 6. References

| Document / path | Role |
|-----------------|------|
| `docs/CLI-surface-implementation-plan.md` | Original unification phases |
| `docs/legacy-parity.md` | Old → new command map (optimistic “done” flags) |
| `docs/cli.md` | User-facing CLI examples |
| `tests/import_fit.rs` | Zepp FIT E2E |
| `tests/fixtures/Zepp20260710164935.fit` | FIT fixture |
| Sibling tools under `/home/aner/rust/{repslog,nutlog,bodylog,bodydashboard}` | Behavior sources |

---

## 7. Status of this report

- **Generated:** 2026-07-10  
- **Location:** `reports/2026-07-10-legacy-parity-gaps.md`  
- **Update when:** a listed gap is closed or accepted as permanent non-goal; adjust scorecard and strike through closed items.  
