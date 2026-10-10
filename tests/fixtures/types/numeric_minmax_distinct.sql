-- MIN/MAX/COUNT(DISTINCT) over NUMERIC: value-based (1.5 and 1.50 are one group).
-- @setup
CREATE TABLE t (a NUMERIC);
INSERT INTO t VALUES (1.5), (1.50), (2.25), (-3.0), (NULL);
-- @test
SELECT MIN(a) AS lo, MAX(a) AS hi, COUNT(DISTINCT a) AS distinct_n FROM t;
