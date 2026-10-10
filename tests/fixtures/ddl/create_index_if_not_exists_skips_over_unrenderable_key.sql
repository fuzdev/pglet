-- The same rule as `create_index_if_not_exists_skips_over_container_key.sql`, for the
-- other pglet-only restriction on this statement: an index expression the deparser
-- cannot render. Postgres **succeeds** here — the canonical idempotent-migration
-- idiom skips on the taken name — and 18.6 has no error for an interval literal
-- anywhere, so the render probe must run after the name conflict and the skip
-- (beside `validate_btree_key_type`) rather than with the three checks that stand in
-- for real 18.6 errors. Run with those it answered `0A000` here.
--
-- The count is what proves the skip skipped rather than building a second index.
-- @setup
CREATE TABLE iv_ine (iv INTERVAL, n INT);
CREATE INDEX iv_ine_taken ON iv_ine (n);
-- @test
CREATE INDEX IF NOT EXISTS iv_ine_taken ON iv_ine ((iv + interval '1 day'));
SELECT count(*)::INT AS n FROM pg_indexes WHERE tablename = 'iv_ine';
