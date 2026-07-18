-- Split micro vs macro nutrients (schema v6).
-- Applied in Rust (PRAGMA user_version 6): apply_v6_split_micro_macro_nutrients.
--
-- Goals:
--   1. Macronutrients live only as columns on product_nutritions (no catalog).
--   2. Micronutrient catalog is micronutrients (was nutrients).
--   3. Extended fats/cholesterol/added sugars that were stored as
--      product_micronutrients rows are promoted to columns.
--
-- product_nutritions gains:
--   saturated_fat_g, trans_fat_g, monounsaturated_fat_g,
--   polyunsaturated_fat_g, cholesterol_mg, added_sugars_g
--
-- Target catalog:
CREATE TABLE IF NOT EXISTS micronutrients (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    unit TEXT NOT NULL,
    recommended_intake REAL,
    created_at TEXT NOT NULL
);

-- Target junction (replaces nutrient_id):
CREATE TABLE IF NOT EXISTS product_micronutrients (
    product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    micronutrient_id INTEGER NOT NULL REFERENCES micronutrients(id),
    amount REAL NOT NULL,
    unit TEXT NOT NULL,
    PRIMARY KEY (product_id, micronutrient_id)
);

-- Macro catalog names removed (case-insensitive):
--   Protein, Carbohydrates, Fat, Fiber, Sugars,
--   Saturated Fat, Trans Fat, Monounsaturated Fat, Polyunsaturated Fat,
--   Cholesterol, Added Sugars
