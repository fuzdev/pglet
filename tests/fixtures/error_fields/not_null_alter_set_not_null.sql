-- `ALTER TABLE … SET NOT NULL` over an existing null is 18.6's `ATRewriteTable` route:
-- the same `23502` with a different message and **no** DETAIL — there is no new row.
-- @setup
CREATE TABLE t_snn (x INT, y TEXT);
INSERT INTO t_snn VALUES (NULL, 'q');

-- @test
-- @expect-error: an existing row holds a null
-- @expect-message
ALTER TABLE t_snn ALTER COLUMN x SET NOT NULL;
