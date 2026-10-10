-- NUMERIC through varied predicate/expression contexts: BETWEEN, IN, CASE,
-- COALESCE, and a DISTINCT projection (value-based dedup).
-- @setup
CREATE TABLE t (id INT, a NUMERIC(10,2));
INSERT INTO t VALUES (1, 10.50), (2, 20.00), (3, 10.50), (4, NULL), (5, 5.25);
-- @test
SELECT DISTINCT
  a,
  a BETWEEN 5 AND 15 AS in_range,
  a IN (10.50, 5.25) AS in_set,
  CASE WHEN a > 15 THEN 'big' ELSE 'small' END AS bucket,
  COALESCE(a, 0) AS with_default
FROM t ORDER BY a;
