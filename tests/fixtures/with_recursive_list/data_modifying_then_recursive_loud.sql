-- A recursive CTE resolved after a data-modifying one would read that write, where
-- 18.6 reads the statement's snapshot: refused rather than answered.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: pglet refuses a recursive CTE after a write (18.6 answers)
-- @test
WITH RECURSIVE w AS (INSERT INTO s VALUES (9) RETURNING x),
  r(n) AS (SELECT x FROM s UNION ALL SELECT n + 100 FROM r WHERE n < 50)
SELECT n FROM r ORDER BY n;
