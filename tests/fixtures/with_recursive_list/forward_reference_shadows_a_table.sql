-- A forward reference to a CTE named like a base table reads the CTE: under
-- `RECURSIVE` the name is in scope for every body. Parsed in declaration order, the
-- reference read the base table's rows instead — a silent wrong answer.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE q AS (SELECT * FROM s),
  s AS (SELECT 5 AS x)
SELECT x FROM q ORDER BY x;
