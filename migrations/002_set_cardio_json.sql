-- Cardio JSON columns for HR zones and laps (repslog parity)
ALTER TABLE exercise_sets ADD COLUMN heart_rate_zones TEXT;
ALTER TABLE exercise_sets ADD COLUMN laps TEXT;
