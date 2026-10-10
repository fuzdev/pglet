-- A `NULL` operand of an `IN` list still evaluates its items, and an item holding an
-- aggregate is evaluated in a grouping query like any other: the `CASE` drops its `max`
-- arm, and the `ELSE` divides by `y = 0`. Skipping every item that holds an aggregate
-- would answer `NULL`.
-- @setup
CREATE TABLE t (x INT, y INT);
INSERT INTO t VALUES (NULL, 0);
-- @expect-error: division by zero
-- @test
SELECT x IN (1, CASE WHEN false THEN max(x) ELSE 1/y END) FROM t GROUP BY x, y;
