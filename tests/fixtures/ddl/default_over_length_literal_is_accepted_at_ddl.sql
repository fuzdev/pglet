-- The acceptance half of `default_length_typmod_is_enforced_per_row.sql`, for the
-- same reason: the per-row `22001` would be indistinguishable from a DDL-time one by
-- SQLSTATE alone, so the pair needs a success on this side.
-- @setup
CREATE TABLE def_len_ok (a INT, b VARCHAR(3) DEFAULT 'abcdef');
-- @test
SELECT column_default IS NOT NULL AS has_default
FROM information_schema.columns WHERE table_name = 'def_len_ok' AND column_name = 'b';
