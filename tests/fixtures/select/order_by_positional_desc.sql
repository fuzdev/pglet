-- A positional `ORDER BY 2 DESC` orders by the second output column (`b`)
-- descending — covering DESC and a non-1 ordinal on the sort-above path.
-- @setup
CREATE TABLE t (a INT, b INT, c INT);
INSERT INTO t VALUES (100, 5, 2), (200, 9, 1), (300, 7, 1);

-- @test
SELECT a, b FROM t ORDER BY 2 DESC;
