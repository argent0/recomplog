-- Product merge alias model (schema v9).
-- Applied in Rust: apply_v9_product_merge_alias.
--
-- Catalog merge retires source products instead of DELETE + event FK rewrite.
-- Event rows keep their original product_id; sources point at the keeper via
-- merged_into_id and carry retired_at (storage time).
-- See reports/append/S2-product-merge-rewrites-event-fks.md.

ALTER TABLE products ADD COLUMN merged_into_id INTEGER REFERENCES products(id);
ALTER TABLE products ADD COLUMN retired_at TEXT;
CREATE INDEX IF NOT EXISTS idx_products_merged_into ON products(merged_into_id);
