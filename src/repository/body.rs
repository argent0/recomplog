use crate::error::{RecomplogError, Result};
use crate::models::{Measurement, MeasurementPoint, Sleep, UserProfile};
use crate::sanity::PreviousMetrics;
use crate::{db::now_utc, utils::make_timestamp_info};
use rusqlite::{params, Connection, OptionalExtension, Row};

pub struct Repository {
    conn: Connection,
}

impl Repository {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    /// Borrow the underlying connection (catalog health checks, etc.).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    fn row_to_measurement(row: &Row) -> rusqlite::Result<Measurement> {
        let id: i64 = row.get(0)?;
        let date: String = row.get(1)?;
        let weight_kg: Option<f64> = row.get(2)?;
        let body_fat_pct: Option<f64> = row.get(3)?;
        let skeletal_muscle_pct: Option<f64> = row.get(4)?;
        let visceral_fat_level: Option<i64> = row.get(5)?;
        let bmi: Option<f64> = row.get(6)?;
        let resting_metabolism_kcal: Option<i64> = row.get(7)?;
        let created_at: String = row.get(8)?;
        let updated_at: String = row.get(9)?;

        Ok(Measurement {
            id,
            date,
            weight_kg,
            body_fat_pct,
            skeletal_muscle_pct,
            visceral_fat_level,
            bmi,
            resting_metabolism_kcal,
            created_at: make_timestamp_info(&created_at),
            updated_at: make_timestamp_info(&updated_at),
        })
    }

    fn row_to_sleep(row: &Row) -> rusqlite::Result<Sleep> {
        let id: i64 = row.get(0)?;
        let date: String = row.get(1)?;
        let bedtime: Option<String> = row.get(2)?;
        let wake_time: Option<String> = row.get(3)?;
        let time_in_bed_minutes: Option<i64> = row.get(4)?;
        let total_sleep_minutes: Option<i64> = row.get(5)?;
        let rem_minutes: Option<i64> = row.get(6)?;
        let deep_minutes: Option<i64> = row.get(7)?;
        let light_minutes: Option<i64> = row.get(8)?;
        let awake_minutes: Option<i64> = row.get(9)?;
        let sleep_efficiency_pct: Option<f64> = row.get(10)?;
        let sleep_score: Option<i64> = row.get(11)?;
        let subjective_quality: Option<i64> = row.get(12)?;
        let awakenings: Option<i64> = row.get(13)?;
        let heart_rate_bpm: Option<f64> = row.get(14)?;
        let hypopnea_per_hr: Option<f64> = row.get(15)?;
        let respiratory_rate: Option<f64> = row.get(16)?;
        let notes: Option<String> = row.get(17)?;
        let created_at: String = row.get(18)?;
        let updated_at: String = row.get(19)?;

        Ok(Sleep {
            id,
            date,
            bedtime,
            wake_time,
            time_in_bed_minutes,
            total_sleep_minutes,
            rem_minutes,
            deep_minutes,
            light_minutes,
            awake_minutes,
            sleep_efficiency_pct,
            sleep_score,
            subjective_quality,
            awakenings,
            heart_rate_bpm,
            hypopnea_per_hr,
            respiratory_rate,
            notes,
            created_at: make_timestamp_info(&created_at),
            updated_at: make_timestamp_info(&updated_at),
        })
    }

    /// Shared SELECT column list for sleep (order matches `row_to_sleep`).
    const SLEEP_SELECT_COLS: &'static str =
        "id, date, bedtime, wake_time, time_in_bed_minutes, total_sleep_minutes, \
         rem_minutes, deep_minutes, light_minutes, awake_minutes, \
         sleep_efficiency_pct, sleep_score, subjective_quality, awakenings, \
         heart_rate_bpm, hypopnea_per_hr, respiratory_rate, notes, \
         created_at, updated_at";

    /// Shared SELECT column list for measurements (order matches `row_to_measurement`).
    const MEASUREMENT_SELECT_COLS: &'static str =
        "id, date, weight_kg, body_fat_pct, skeletal_muscle_pct, visceral_fat_level, \
         bmi, resting_metabolism_kcal, created_at, updated_at";

    /// Create a new measurement sample (always appends; multiple rows per date allowed).
    /// Returns the new id.
    #[allow(clippy::too_many_arguments)]
    pub fn create_measurement(
        &self,
        date: &str,
        weight_kg: Option<f64>,
        body_fat_pct: Option<f64>,
        skeletal_muscle_pct: Option<f64>,
        visceral_fat_level: Option<i64>,
        bmi: Option<f64>,
        resting_metabolism_kcal: Option<i64>,
    ) -> Result<i64> {
        let now = now_utc();
        self.conn.execute(
            "INSERT INTO measurements
             (date, weight_kg, body_fat_pct, skeletal_muscle_pct, visceral_fat_level, bmi, resting_metabolism_kcal, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                date,
                weight_kg,
                body_fat_pct,
                skeletal_muscle_pct,
                visceral_fat_level,
                bmi,
                resting_metabolism_kcal,
                now
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List measurements in [since, until] (inclusive), or all if None.
    /// Sorted by date DESC, then created_at DESC, id DESC (newest samples first).
    pub fn list_measurements(
        &self,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<Measurement>> {
        let cols = Self::MEASUREMENT_SELECT_COLS;
        let order = "ORDER BY date DESC, created_at DESC, id DESC";
        let rows = match (since, until) {
            (None, None) => {
                let sql = format!("SELECT {cols} FROM measurements {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([], Self::row_to_measurement)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), None) => {
                let sql = format!("SELECT {cols} FROM measurements WHERE date >= ?1 {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s], Self::row_to_measurement)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (None, Some(u)) => {
                let sql = format!("SELECT {cols} FROM measurements WHERE date <= ?1 {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([u], Self::row_to_measurement)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), Some(u)) => {
                let sql = format!(
                    "SELECT {cols} FROM measurements WHERE date >= ?1 AND date <= ?2 {order}"
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s, u], Self::row_to_measurement)?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };
        Ok(rows)
    }

    /// Get a single measurement by id.
    pub fn get_measurement(&self, id: i64) -> Result<Measurement> {
        let m: Option<Measurement> = self
            .conn
            .query_row(
                "SELECT id, date, weight_kg, body_fat_pct, skeletal_muscle_pct, visceral_fat_level, bmi, resting_metabolism_kcal, created_at, updated_at
                 FROM measurements WHERE id = ?1",
                [id],
                Self::row_to_measurement,
            )
            .optional()?;
        m.ok_or(RecomplogError::MeasurementNotFound(id))
    }

    /// Latest measurement sample for a calendar date (`created_at DESC, id DESC`).
    /// Multiple samples per day are allowed; this picks the last-written one.
    pub fn get_measurement_by_date(&self, date: &str) -> Result<Measurement> {
        let cols = Self::MEASUREMENT_SELECT_COLS;
        let sql = format!(
            "SELECT {cols} FROM measurements WHERE date = ?1 \
             ORDER BY created_at DESC, id DESC LIMIT 1"
        );
        let m: Option<Measurement> = self
            .conn
            .query_row(&sql, [date], Self::row_to_measurement)
            .optional()?;
        m.ok_or(RecomplogError::MeasurementNotFoundForDate(date.to_string()))
    }

    fn count_measurements_for_date(&self, date: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM measurements WHERE date = ?1",
            [date],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Sole measurement id for date when mutating by date. Errors if 0 or >1 rows.
    fn sole_measurement_id_for_date(&self, date: &str) -> Result<i64> {
        let count = self.count_measurements_for_date(date)?;
        if count == 0 {
            return Err(RecomplogError::MeasurementNotFoundForDate(date.to_string()));
        }
        if count > 1 {
            return Err(RecomplogError::MeasurementAmbiguousForDate {
                date: date.to_string(),
                count,
            });
        }
        let id: i64 =
            self.conn
                .query_row("SELECT id FROM measurements WHERE date = ?1", [date], |r| {
                    r.get(0)
                })?;
        Ok(id)
    }

    /// Update fields on an existing measurement (by id). Only non-None fields are changed.
    /// Refreshes updated_at.
    #[allow(clippy::too_many_arguments)]
    pub fn update_measurement(
        &self,
        id: i64,
        weight_kg: Option<f64>,
        body_fat_pct: Option<f64>,
        skeletal_muscle_pct: Option<f64>,
        visceral_fat_level: Option<i64>,
        bmi: Option<f64>,
        resting_metabolism_kcal: Option<i64>,
    ) -> Result<()> {
        // Ensure exists
        let _ = self.get_measurement(id)?;

        let now = now_utc();

        // Build dynamic update. For a small fixed schema this is acceptable.
        // We always touch updated_at.
        let mut sets: Vec<String> = vec!["updated_at = ?".to_string()];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now)];

        if let Some(v) = weight_kg {
            sets.push("weight_kg = ?".to_string());
            params.push(Box::new(v));
        }
        if let Some(v) = body_fat_pct {
            sets.push("body_fat_pct = ?".to_string());
            params.push(Box::new(v));
        }
        if let Some(v) = skeletal_muscle_pct {
            sets.push("skeletal_muscle_pct = ?".to_string());
            params.push(Box::new(v));
        }
        if let Some(v) = visceral_fat_level {
            sets.push("visceral_fat_level = ?".to_string());
            params.push(Box::new(v));
        }
        if let Some(v) = bmi {
            sets.push("bmi = ?".to_string());
            params.push(Box::new(v));
        }
        if let Some(v) = resting_metabolism_kcal {
            sets.push("resting_metabolism_kcal = ?".to_string());
            params.push(Box::new(v));
        }

        let sql = format!("UPDATE measurements SET {} WHERE id = ?", sets.join(", "));
        // Append id as last param
        params.push(Box::new(id));

        // rusqlite wants &[&dyn ToSql] for execute. We convert.
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let affected = self.conn.execute(&sql, &param_refs[..])?;
        if affected == 0 {
            return Err(RecomplogError::MeasurementNotFound(id));
        }
        Ok(())
    }

    /// Update by date when exactly one sample exists. Returns the updated id.
    /// If multiple samples share the date, fails with `MeasurementAmbiguousForDate`.
    #[allow(clippy::too_many_arguments)]
    pub fn update_measurement_by_date(
        &self,
        date: &str,
        weight_kg: Option<f64>,
        body_fat_pct: Option<f64>,
        skeletal_muscle_pct: Option<f64>,
        visceral_fat_level: Option<i64>,
        bmi: Option<f64>,
        resting_metabolism_kcal: Option<i64>,
    ) -> Result<i64> {
        let id = self.sole_measurement_id_for_date(date)?;
        self.update_measurement(
            id,
            weight_kg,
            body_fat_pct,
            skeletal_muscle_pct,
            visceral_fat_level,
            bmi,
            resting_metabolism_kcal,
        )?;
        Ok(id)
    }

    /// Delete by id. Returns the deleted id on success.
    pub fn delete_measurement(&self, id: i64) -> Result<i64> {
        let affected = self
            .conn
            .execute("DELETE FROM measurements WHERE id = ?1", [id])?;
        if affected == 0 {
            return Err(RecomplogError::MeasurementNotFound(id));
        }
        Ok(id)
    }

    /// Delete by date when exactly one sample exists. Returns the deleted id.
    /// If multiple samples share the date, fails with `MeasurementAmbiguousForDate`.
    pub fn delete_measurement_by_date(&self, date: &str) -> Result<i64> {
        let id = self.sole_measurement_id_for_date(date)?;
        self.conn
            .execute("DELETE FROM measurements WHERE id = ?1", [id])?;
        Ok(id)
    }

    /// Latest non-null value for each metric with `date < before_date` (YYYY-MM-DD).
    /// Each field is resolved independently so partial historical rows still contribute.
    pub fn get_previous_metric_values(&self, before_date: &str) -> Result<PreviousMetrics> {
        Ok(PreviousMetrics {
            weight_kg: self.latest_f64_before("weight_kg", before_date)?,
            body_fat_pct: self.latest_f64_before("body_fat_pct", before_date)?,
            skeletal_muscle_pct: self.latest_f64_before("skeletal_muscle_pct", before_date)?,
            visceral_fat_level: self
                .latest_i64_before("visceral_fat_level", before_date)?
                .map(|(d, v)| (d, v as f64)),
            bmi: self.latest_f64_before("bmi", before_date)?,
            resting_metabolism_kcal: self
                .latest_i64_before("resting_metabolism_kcal", before_date)?
                .map(|(d, v)| (d, v as f64)),
        })
    }

    fn latest_f64_before(&self, column: &str, before_date: &str) -> Result<Option<(String, f64)>> {
        // column is a fixed identifier from call sites, not user input
        let sql = format!(
            "SELECT date, {column} FROM measurements \
             WHERE date < ?1 AND {column} IS NOT NULL \
             ORDER BY date DESC, created_at DESC, id DESC LIMIT 1"
        );
        let row: Option<(String, f64)> = self
            .conn
            .query_row(&sql, [before_date], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(row)
    }

    fn latest_i64_before(&self, column: &str, before_date: &str) -> Result<Option<(String, i64)>> {
        let sql = format!(
            "SELECT date, {column} FROM measurements \
             WHERE date < ?1 AND {column} IS NOT NULL \
             ORDER BY date DESC, created_at DESC, id DESC LIMIT 1"
        );
        let row: Option<(String, i64)> = self
            .conn
            .query_row(&sql, [before_date], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(row)
    }

    // ---------- User profile (height, date of birth) via `config` entity ----------

    /// Load the singleton user profile. Returns empty profile (all None) if never set.
    pub fn get_profile(&self) -> Result<UserProfile> {
        let row: Option<(Option<f64>, Option<String>, String)> = self
            .conn
            .query_row(
                "SELECT height_cm, date_of_birth, updated_at FROM user_profile WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        match row {
            Some((height_cm, date_of_birth, updated)) => Ok(UserProfile {
                height_cm,
                date_of_birth,
                updated_at: Some(make_timestamp_info(&updated)),
            }),
            None => Ok(UserProfile {
                height_cm: None,
                date_of_birth: None,
                updated_at: None,
            }),
        }
    }

    /// Partial update of the user profile. Only provided fields are changed.
    /// Creates the profile row on first set. Always refreshes updated_at.
    pub fn set_profile(&self, height_cm: Option<f64>, date_of_birth: Option<String>) -> Result<()> {
        // Basic validation for height if being set (or already present after merge)
        if let Some(h) = height_cm {
            if !(h > 0.0 && h.is_finite()) {
                return Err(RecomplogError::InvalidProfile(
                    "height_cm must be a positive number".to_string(),
                ));
            }
        }

        let current = self.get_profile()?;
        let new_height = height_cm.or(current.height_cm);
        let new_dob = date_of_birth.or(current.date_of_birth);

        // Validate merged height too (in case we kept an old bad value, though we shouldn't have)
        if let Some(h) = new_height {
            if !(h > 0.0 && h.is_finite()) {
                return Err(RecomplogError::InvalidProfile(
                    "height_cm must be a positive number".to_string(),
                ));
            }
        }

        let now = now_utc();
        self.conn.execute(
            "INSERT OR REPLACE INTO user_profile (id, height_cm, date_of_birth, updated_at)
             VALUES (1, ?, ?, ?)",
            params![new_height, new_dob, now],
        )?;
        Ok(())
    }

    /// Fetch one measurement point per calendar day (last by `created_at`, then `id`),
    /// ordered by date ASC. Used by day-series reports and charts.
    pub fn get_measurements_for_report(
        &self,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MeasurementPoint>> {
        // Correlated subquery: last-written sample per event date.
        let base = "SELECT date, weight_kg, body_fat_pct, skeletal_muscle_pct, \
                    visceral_fat_level, bmi, resting_metabolism_kcal \
                    FROM measurements m \
                    WHERE id = ( \
                        SELECT id FROM measurements m2 \
                        WHERE m2.date = m.date \
                        ORDER BY m2.created_at DESC, m2.id DESC LIMIT 1 \
                    )";

        let rows: Vec<MeasurementPoint> = match (since, until) {
            (None, None) => {
                let sql = format!("{base} ORDER BY date ASC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([], Self::row_to_measurement_point)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), None) => {
                let sql = format!("{base} AND date >= ?1 ORDER BY date ASC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s], Self::row_to_measurement_point)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (None, Some(u)) => {
                let sql = format!("{base} AND date <= ?1 ORDER BY date ASC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([u], Self::row_to_measurement_point)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), Some(u)) => {
                let sql = format!("{base} AND date >= ?1 AND date <= ?2 ORDER BY date ASC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s, u], Self::row_to_measurement_point)?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };
        Ok(rows)
    }

    fn row_to_measurement_point(row: &Row) -> rusqlite::Result<MeasurementPoint> {
        Ok(MeasurementPoint {
            date: row.get(0)?,
            weight_kg: row.get(1)?,
            body_fat_pct: row.get(2)?,
            skeletal_muscle_pct: row.get(3)?,
            visceral_fat_level: row.get(4)?,
            bmi: row.get(5)?,
            resting_metabolism_kcal: row.get(6)?,
        })
    }

    // ---------- Sleep (sleep) per spec/02-sleep-logging.md ----------

    /// Create a new sleep sample (always appends; multiple rows per wake-up date allowed).
    /// Returns the new id.
    #[allow(clippy::too_many_arguments)]
    pub fn create_sleep(
        &self,
        date: &str,
        bedtime: Option<&str>,
        wake_time: Option<&str>,
        time_in_bed_minutes: Option<i64>,
        total_sleep_minutes: Option<i64>,
        rem_minutes: Option<i64>,
        deep_minutes: Option<i64>,
        light_minutes: Option<i64>,
        awake_minutes: Option<i64>,
        sleep_efficiency_pct: Option<f64>,
        sleep_score: Option<i64>,
        subjective_quality: Option<i64>,
        awakenings: Option<i64>,
        heart_rate_bpm: Option<f64>,
        hypopnea_per_hr: Option<f64>,
        respiratory_rate: Option<f64>,
        notes: Option<&str>,
    ) -> Result<i64> {
        let now = now_utc();
        self.conn.execute(
            "INSERT INTO sleep
             (date, bedtime, wake_time, time_in_bed_minutes, total_sleep_minutes,
              rem_minutes, deep_minutes, light_minutes, awake_minutes,
              sleep_efficiency_pct, sleep_score, subjective_quality, awakenings,
              heart_rate_bpm, hypopnea_per_hr, respiratory_rate, notes,
              created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?18)",
            params![
                date,
                bedtime,
                wake_time,
                time_in_bed_minutes,
                total_sleep_minutes,
                rem_minutes,
                deep_minutes,
                light_minutes,
                awake_minutes,
                sleep_efficiency_pct,
                sleep_score,
                subjective_quality,
                awakenings,
                heart_rate_bpm,
                hypopnea_per_hr,
                respiratory_rate,
                notes,
                now
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List sleep sessions in [since, until] (inclusive), or all if None.
    /// Sorted by date DESC, then created_at DESC, id DESC.
    pub fn list_sleeps(&self, since: Option<&str>, until: Option<&str>) -> Result<Vec<Sleep>> {
        let cols = Self::SLEEP_SELECT_COLS;
        let order = "ORDER BY date DESC, created_at DESC, id DESC";
        let rows = match (since, until) {
            (None, None) => {
                let sql = format!("SELECT {cols} FROM sleep {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([], Self::row_to_sleep)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), None) => {
                let sql = format!("SELECT {cols} FROM sleep WHERE date >= ?1 {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s], Self::row_to_sleep)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (None, Some(u)) => {
                let sql = format!("SELECT {cols} FROM sleep WHERE date <= ?1 {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([u], Self::row_to_sleep)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), Some(u)) => {
                let sql =
                    format!("SELECT {cols} FROM sleep WHERE date >= ?1 AND date <= ?2 {order}");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s, u], Self::row_to_sleep)?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };
        Ok(rows)
    }

    /// Get a single sleep record by id.
    pub fn get_sleep(&self, id: i64) -> Result<Sleep> {
        let sql = format!(
            "SELECT {} FROM sleep WHERE id = ?1",
            Self::SLEEP_SELECT_COLS
        );
        let s: Option<Sleep> = self
            .conn
            .query_row(&sql, [id], Self::row_to_sleep)
            .optional()?;
        s.ok_or(RecomplogError::SleepNotFound(id))
    }

    /// Latest sleep sample for a wake-up date (`created_at DESC, id DESC`).
    /// Multiple samples per day are allowed; this picks the last-written one.
    pub fn get_sleep_by_date(&self, date: &str) -> Result<Sleep> {
        let sql = format!(
            "SELECT {} FROM sleep WHERE date = ?1 \
             ORDER BY created_at DESC, id DESC LIMIT 1",
            Self::SLEEP_SELECT_COLS
        );
        let s: Option<Sleep> = self
            .conn
            .query_row(&sql, [date], Self::row_to_sleep)
            .optional()?;
        s.ok_or(RecomplogError::SleepNotFoundForDate(date.to_string()))
    }

    fn count_sleeps_for_date(&self, date: &str) -> Result<i64> {
        let n: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM sleep WHERE date = ?1", [date], |r| {
                    r.get(0)
                })?;
        Ok(n)
    }

    /// Sole sleep id for date when mutating by date. Errors if 0 or >1 rows.
    fn sole_sleep_id_for_date(&self, date: &str) -> Result<i64> {
        let count = self.count_sleeps_for_date(date)?;
        if count == 0 {
            return Err(RecomplogError::SleepNotFoundForDate(date.to_string()));
        }
        if count > 1 {
            return Err(RecomplogError::SleepAmbiguousForDate {
                date: date.to_string(),
                count,
            });
        }
        let id: i64 = self
            .conn
            .query_row("SELECT id FROM sleep WHERE date = ?1", [date], |r| r.get(0))?;
        Ok(id)
    }

    /// Update fields on an existing sleep record (by id). Only non-None fields are changed.
    /// Refreshes updated_at. Partial updates supported.
    #[allow(clippy::too_many_arguments)]
    pub fn update_sleep(
        &self,
        id: i64,
        bedtime: Option<&str>,
        wake_time: Option<&str>,
        time_in_bed_minutes: Option<i64>,
        total_sleep_minutes: Option<i64>,
        rem_minutes: Option<i64>,
        deep_minutes: Option<i64>,
        light_minutes: Option<i64>,
        awake_minutes: Option<i64>,
        sleep_efficiency_pct: Option<f64>,
        sleep_score: Option<i64>,
        subjective_quality: Option<i64>,
        awakenings: Option<i64>,
        heart_rate_bpm: Option<f64>,
        hypopnea_per_hr: Option<f64>,
        respiratory_rate: Option<f64>,
        notes: Option<&str>,
    ) -> Result<()> {
        // Ensure exists
        let _ = self.get_sleep(id)?;

        let now = now_utc();

        let mut sets: Vec<String> = vec!["updated_at = ?".to_string()];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now)];

        macro_rules! set_opt_str {
            ($field:ident, $col:literal) => {
                if let Some(v) = $field {
                    sets.push(format!("{} = ?", $col));
                    params.push(Box::new(v.to_string()));
                }
            };
        }
        macro_rules! set_opt_i64 {
            ($field:ident, $col:literal) => {
                if let Some(v) = $field {
                    sets.push(format!("{} = ?", $col));
                    params.push(Box::new(v));
                }
            };
        }
        macro_rules! set_opt_f64 {
            ($field:ident, $col:literal) => {
                if let Some(v) = $field {
                    sets.push(format!("{} = ?", $col));
                    params.push(Box::new(v));
                }
            };
        }

        set_opt_str!(bedtime, "bedtime");
        set_opt_str!(wake_time, "wake_time");
        set_opt_i64!(time_in_bed_minutes, "time_in_bed_minutes");
        set_opt_i64!(total_sleep_minutes, "total_sleep_minutes");
        set_opt_i64!(rem_minutes, "rem_minutes");
        set_opt_i64!(deep_minutes, "deep_minutes");
        set_opt_i64!(light_minutes, "light_minutes");
        set_opt_i64!(awake_minutes, "awake_minutes");
        set_opt_f64!(sleep_efficiency_pct, "sleep_efficiency_pct");
        set_opt_i64!(sleep_score, "sleep_score");
        set_opt_i64!(subjective_quality, "subjective_quality");
        set_opt_i64!(awakenings, "awakenings");
        set_opt_f64!(heart_rate_bpm, "heart_rate_bpm");
        set_opt_f64!(hypopnea_per_hr, "hypopnea_per_hr");
        set_opt_f64!(respiratory_rate, "respiratory_rate");
        set_opt_str!(notes, "notes");

        let sql = format!("UPDATE sleep SET {} WHERE id = ?", sets.join(", "));
        params.push(Box::new(id));

        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let affected = self.conn.execute(&sql, &param_refs[..])?;
        if affected == 0 {
            return Err(RecomplogError::SleepNotFound(id));
        }
        Ok(())
    }

    /// Update by date when exactly one sample exists. Returns the updated id.
    /// If multiple samples share the date, fails with `SleepAmbiguousForDate`.
    #[allow(clippy::too_many_arguments)]
    pub fn update_sleep_by_date(
        &self,
        date: &str,
        bedtime: Option<&str>,
        wake_time: Option<&str>,
        time_in_bed_minutes: Option<i64>,
        total_sleep_minutes: Option<i64>,
        rem_minutes: Option<i64>,
        deep_minutes: Option<i64>,
        light_minutes: Option<i64>,
        awake_minutes: Option<i64>,
        sleep_efficiency_pct: Option<f64>,
        sleep_score: Option<i64>,
        subjective_quality: Option<i64>,
        awakenings: Option<i64>,
        heart_rate_bpm: Option<f64>,
        hypopnea_per_hr: Option<f64>,
        respiratory_rate: Option<f64>,
        notes: Option<&str>,
    ) -> Result<i64> {
        let id = self.sole_sleep_id_for_date(date)?;
        self.update_sleep(
            id,
            bedtime,
            wake_time,
            time_in_bed_minutes,
            total_sleep_minutes,
            rem_minutes,
            deep_minutes,
            light_minutes,
            awake_minutes,
            sleep_efficiency_pct,
            sleep_score,
            subjective_quality,
            awakenings,
            heart_rate_bpm,
            hypopnea_per_hr,
            respiratory_rate,
            notes,
        )?;
        Ok(id)
    }

    /// Delete by id. Returns the deleted id.
    pub fn delete_sleep(&self, id: i64) -> Result<i64> {
        let affected = self.conn.execute("DELETE FROM sleep WHERE id = ?1", [id])?;
        if affected == 0 {
            return Err(RecomplogError::SleepNotFound(id));
        }
        Ok(id)
    }

    /// Delete by date when exactly one sample exists. Returns the deleted id.
    /// If multiple samples share the date, fails with `SleepAmbiguousForDate`.
    pub fn delete_sleep_by_date(&self, date: &str) -> Result<i64> {
        let id = self.sole_sleep_id_for_date(date)?;
        self.conn.execute("DELETE FROM sleep WHERE id = ?1", [id])?;
        Ok(id)
    }

    /// One sleep sample per wake-up date (last by `created_at`, then `id`), date ASC.
    /// Used by day-series sleep reports.
    pub fn get_sleeps_for_report(
        &self,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<Sleep>> {
        let cols = Self::SLEEP_SELECT_COLS;
        let base = format!(
            "SELECT {cols} FROM sleep s \
             WHERE id = ( \
                 SELECT id FROM sleep s2 \
                 WHERE s2.date = s.date \
                 ORDER BY s2.created_at DESC, s2.id DESC LIMIT 1 \
             )"
        );
        let rows: Vec<Sleep> = match (since, until) {
            (None, None) => {
                let sql = format!("{base} ORDER BY date ASC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([], Self::row_to_sleep)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), None) => {
                let sql = format!("{base} AND date >= ?1 ORDER BY date ASC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s], Self::row_to_sleep)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (None, Some(u)) => {
                let sql = format!("{base} AND date <= ?1 ORDER BY date ASC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([u], Self::row_to_sleep)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), Some(u)) => {
                let sql = format!("{base} AND date >= ?1 AND date <= ?2 ORDER BY date ASC");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s, u], Self::row_to_sleep)?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };
        Ok(rows)
    }

    // ---------- Exercise sets (for check audit) ----------

    const SET_AUDIT_SELECT: &str = "
        SELECT s.id,
               s.reps, s.weight_kg, s.external_load_kg, s.distance_km, s.duration_seconds,
               s.rpe, s.rir, s.effective_reps, s.rest_seconds,
               s.avg_heart_rate_bpm, s.max_heart_rate_bpm, s.avg_pace_min_per_km,
               s.calories_burned, s.avg_cadence_spm, s.total_ascent_m, s.total_descent_m,
               s.heart_rate_zones, s.laps,
               date(w.started_at, 'localtime'), e.name
        FROM exercise_sets s
        JOIN workout_exercises we ON we.id = s.workout_exercise_id
        JOIN workouts w ON w.id = we.workout_id
        JOIN exercises e ON e.id = we.exercise_id
    ";

    fn row_to_set_audit(row: &Row) -> rusqlite::Result<SetAuditRow> {
        Ok(SetAuditRow {
            id: row.get(0)?,
            reps: row.get(1)?,
            weight_kg: row.get(2)?,
            external_load_kg: row.get(3)?,
            distance_km: row.get(4)?,
            duration_seconds: row.get(5)?,
            rpe: row.get(6)?,
            rir: row.get(7)?,
            effective_reps: row.get(8)?,
            rest_seconds: row.get(9)?,
            avg_heart_rate_bpm: row.get(10)?,
            max_heart_rate_bpm: row.get(11)?,
            avg_pace_min_per_km: row.get(12)?,
            calories_burned: row.get(13)?,
            avg_cadence_spm: row.get(14)?,
            total_ascent_m: row.get(15)?,
            total_descent_m: row.get(16)?,
            heart_rate_zones: row.get(17)?,
            laps: row.get(18)?,
            workout_date: row.get(19)?,
            exercise_name: row.get(20)?,
        })
    }

    /// List exercise sets whose workout session day falls in [since, until] (inclusive),
    /// or all sets if bounds are None. Ordered by workout date then set id.
    pub fn list_exercise_sets_for_check(
        &self,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<SetAuditRow>> {
        let base = Self::SET_AUDIT_SELECT;
        let rows = match (since, until) {
            (None, None) => {
                let sql = format!("{} ORDER BY date(w.started_at, 'localtime'), s.id", base);
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([], Self::row_to_set_audit)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), None) => {
                let sql = format!(
                    "{} WHERE date(w.started_at, 'localtime') >= ?1 ORDER BY date(w.started_at, 'localtime'), s.id",
                    base
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s], Self::row_to_set_audit)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (None, Some(u)) => {
                let sql = format!(
                    "{} WHERE date(w.started_at, 'localtime') <= ?1 ORDER BY date(w.started_at, 'localtime'), s.id",
                    base
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([u], Self::row_to_set_audit)?;
                rows.filter_map(|r| r.ok()).collect()
            }
            (Some(s), Some(u)) => {
                let sql = format!(
                    "{} WHERE date(w.started_at, 'localtime') >= ?1 AND date(w.started_at, 'localtime') <= ?2 \
                     ORDER BY date(w.started_at, 'localtime'), s.id",
                    base
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map([s, u], Self::row_to_set_audit)?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };
        Ok(rows)
    }
}

/// One exercise set row for historical `check` audit (joined to workout date + exercise name).
#[derive(Debug, Clone)]
pub struct SetAuditRow {
    pub id: i64,
    pub reps: Option<i32>,
    pub weight_kg: Option<f64>,
    pub external_load_kg: Option<f64>,
    pub distance_km: Option<f64>,
    pub duration_seconds: Option<i32>,
    pub rpe: Option<f64>,
    pub rir: Option<f64>,
    pub effective_reps: Option<i32>,
    pub rest_seconds: Option<i32>,
    pub avg_heart_rate_bpm: Option<f64>,
    pub max_heart_rate_bpm: Option<f64>,
    pub avg_pace_min_per_km: Option<f64>,
    pub calories_burned: Option<i32>,
    pub avg_cadence_spm: Option<f64>,
    pub total_ascent_m: Option<f64>,
    pub total_descent_m: Option<f64>,
    /// Raw JSON from DB; parse in the check handler.
    pub heart_rate_zones: Option<String>,
    /// Raw JSON from DB; parse in the check handler.
    pub laps: Option<String>,
    /// Calendar day of the parent workout (`date(w.started_at, 'localtime')`).
    pub workout_date: String,
    pub exercise_name: String,
}
