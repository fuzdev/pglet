-- A `double precision` branch decides a set's target only where every other branch is
-- in the numeric category: beside `upper(tx)`, which no reach types and the walkers read
-- as `text`, `nullif` is left as written and its `=` is refused. Cast to the float
-- instead, the first argument answered `22P02`, or a row.
-- @setup
CREATE TABLE t (k INT, tx TEXT, f FLOAT8);
INSERT INTO t VALUES (1, '7', 7), (2, 'x', 2.5);
-- @expect-error: operator does not exist: text = double precision (42883)
-- @test
SELECT k, nullif(upper(tx), f + 0) FROM t ORDER BY k;
