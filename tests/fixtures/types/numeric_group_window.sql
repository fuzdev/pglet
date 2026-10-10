-- GROUP BY a NUMERIC column groups by value (1.50 and 1.5 are one group),
-- and a numeric aggregate over each group is exact.
-- @setup
CREATE TABLE t (a NUMERIC, qty INT);
INSERT INTO t VALUES (1.50, 2), (1.5, 3), (2.25, 1), (2.250, 4);
-- @test
SELECT a, SUM(qty) AS total_qty, COUNT(*) AS rows FROM t GROUP BY a ORDER BY a;
