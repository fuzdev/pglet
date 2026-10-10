-- A nested aggregate in `HAVING` is the same `42803`.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT);
INSERT INTO t VALUES (1, 1, 1), (1, 2, 2), (2, 5, 5);
CREATE TABLE u (k INT, i INT);
INSERT INTO u VALUES (1, 10), (2, 20);
-- @expect-error: aggregate function calls cannot be nested
-- @expect-message
-- @test
SELECT k FROM t GROUP BY k HAVING avg(max(i)) > 0;
