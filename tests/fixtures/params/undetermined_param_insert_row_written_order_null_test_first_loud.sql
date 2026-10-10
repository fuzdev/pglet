-- The mirror of `undetermined_param_insert_row_written_order_cast_first`: the null test
-- is written first and its column stored second, so the table's order would put the
-- cast ahead of it.
-- @setup
CREATE TABLE up_kn (k TEXT PRIMARY KEY, v JSONB, n INT);
-- @param $1 text z
-- @profile-lock
-- @test
-- @expect-error: the null test is written before the occurrence that types the parameter: 42P08
-- @expect-message
INSERT INTO up_kn (n, k) VALUES (CASE WHEN $1 IS NULL THEN 0 ELSE 1 END, $1::text);
