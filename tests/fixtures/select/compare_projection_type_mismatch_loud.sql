-- A mistyped comparison in the **projection** (not a WHERE) over a table with no
-- rows. Postgres rejects `SELECT name = 5` at parse time; pglet only ever evaluated
-- the expression per row, so with no rows it reported an empty result. The static
-- gate runs over the resolved projection, so the verdict no longer depends on
-- whether a row exists.
-- @setup
CREATE TABLE t (name TEXT, n INT);
-- @test
-- @expect-error: no text = integer operator in a projection over zero rows
SELECT name = 5 FROM t;
