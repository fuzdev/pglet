-- Same as `distinct_on_position_out_of_range`, but on the WITH-FROM lowering path
-- (`lower_select`): a positional `DISTINCT ON (5)` over a two-column table is still
-- 42P10 (Postgres: "DISTINCT ON position 5 is not in select list").
-- @expect-error: DISTINCT ON position out of range, with FROM (42P10)
-- @setup
CREATE TABLE t (a INT, b INT);
-- @test
SELECT DISTINCT ON (5) * FROM t;
