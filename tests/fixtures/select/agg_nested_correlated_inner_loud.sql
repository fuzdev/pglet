-- `avg(max(t.i))` inside a subquery over `u`, `t` being the enclosing query's: both
-- engines refuse it `42803`, for different reasons. 18.6 assigns both aggregates to the
-- outer query, the only level their arguments name, and one is nested in the other; pglet
-- has no outer-level aggregates and reads both as the subquery's (`EDGES.md` row 303).
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT);
INSERT INTO t VALUES (1, 1, 1), (1, 2, 2), (2, 5, 5);
CREATE TABLE u (k INT, i INT);
INSERT INTO u VALUES (1, 10), (2, 20);
-- @expect-error: aggregate function calls cannot be nested
-- @expect-message
-- @test
SELECT (SELECT avg(max(t.i)) FROM u) FROM t;
