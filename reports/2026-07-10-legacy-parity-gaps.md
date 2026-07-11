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
| `check` (body + sleep) | High for body | **No workout audit** |
| Nutrition CRUD + tags/stores | High | Nutrient seed; edge polish |
| Nutrition reports | Medium | Summary, micros, spending-by, value filters |
| Workout logging (sets) | High | dry-run, manual cadence/elevation, finished_at, strict cardio |
| Workout analysis | Medium–low | PRs, history, progression, track_metrics on show |
| FIT import | High (E2E) | Zone defaults; profile-path less tested |
| Legacy import | Medium–high | Trackpoints, activity_imports, zones/laps from old DBs |
| HTML dashboard | Medium | Regression, sleep stages, full macros, training block |
| Config | Medium–high | Nested `[sanity.measurements]` layout / docs |

**Bottom line:** CRUD and FIT import are largely done. Remaining gaps are mainly **analysis and audit**, **complete legacy provenance migration**, and **report/dashboard depth**.

---

## 1. High impact

### 1.1 `check` does not audit workouts

**Current:** `recomplog check` scans measurements and sleep (absolute limits + optional `--variations` deltas).

**Missing:** Bulk scan of `exercise_sets` against `[sanity.workout]` (weight, reps, HR, distance, duration, zones, etc.).

**Note:** Workout limits **are** applied on set **write**. Historical/imported rows are not re-audited.

**Source parity:** bodylog `check` for body; plan also called for workout domain in unified `check`.

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

### 1.3 Nutrition reports — core only

**Current:**

- `report nutrition list` — consumption rows + approximate scaled macros  
- `report nutrition spending` — sum of `price_cents` over last N days  

**Missing vs nutlog:**

| Feature | nutlog | recomplog |
|---------|--------|-----------|
| `report nutrition summary` (period aggregates) | Yes | No dedicated summary command/shape |
| Per-day list `--value calories\|protein\|…` | Yes | Always broad macro-ish dump |
| Micronutrient period totals | Yes | Not on report path |
| Spending grouped by store / product / month | Yes (`--by`) | Flat total only |
| Strict `--days` vs `--since`/`--until` mutual exclusion | Yes | Looser |

Documented in `docs/legacy-parity.md` as **“done (core)”**, not full.

---

### 1.4 HTML dashboard thinner than bodydashboard

**Current `report html`:** Chart.js page with weight, body fat, sleep minutes, kcal/protein; overview cards including latest fat/lean mass.

**Missing vs bodydashboard:**

- Linear regression + trend labels per body metric  
- Fat mass / lean mass **time series** (not only latest cards)  
- Sleep stages (REM / deep / light / awake), efficiency, score charts  
- Full macro charts (fat, fiber, sugars)  
- Stats helpers (median, confidence) from original `stats.rs`  
- Optional training volume block  
- Layout/CSS fidelity (explicitly lower priority if data domains exist)

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

### 2.2 Legacy import incomplete for workout provenance

**In good shape:** body measurements/sleep; nutrition products/purchases/consumptions/tags/micros/stores; workout skeleton (exercises → workouts → workout_exercises → sets).

**Typically missing on legacy workout import:**

- `activity_imports` (file SHA / device / FIT metadata)  
- `activity_trackpoints`  
- Source `heart_rate_zones` / `laps` columns on sets  

Migrating an old repslog DB can drop GPS/HR samples and import idempotency keys.

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

## 4. Testing and process gaps

| Gap | Detail |
|-----|--------|
| Large uncommitted tree | Parity work (workout, FIT, nutrition, fixtures) may still be uncommitted relative to last clean commit |
| Root FIT duplicate | `Zepp20260710164935.fit` at repo root and under `tests/fixtures/` — fixtures path is canonical for tests |
| No legacy-import E2E | No automated test with real bodylog/nutlog/repslog DB fixtures |
| HTML tests | Only “file exists / contains Chart”; no structure/content regression |
| `check` + workout | No test that historical set outliers surface in `check` |
| FIT zones without CLI bounds | Device zones on Zepp fixture may be null; profile-derived zones less covered |

**FIT E2E that *is* present:** `tests/import_fit.rs` using `tests/fixtures/Zepp20260710164935.fit` (distance, duration, HR, trackpoints, dedup, `--force`, `--hr-zone-bounds`).

---

## 5. Recommended close-out order

1. Extend **`check`** to scan `exercise_sets` with `sanity.workout`.  
2. Port **`track_metrics`** into `workout show` (and optionally HTML/report).  
3. Flesh out **nutrition reports** (summary, micros, spending-by, value filters).  
4. Complete **legacy import** for trackpoints, `activity_imports`, zones/laps.  
5. Expand **stats** (`prs` / `history` / `weight` progression / summary).  
6. Deepen **HTML** (regression, sleep stages, remaining macros).  
7. CLI polish: **`--dry-run`**, set update for JSON fields, manual cadence/ascent, `finished_at`.  
8. Housekeeping: commit, single FIT fixture path, more integration tests.

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
