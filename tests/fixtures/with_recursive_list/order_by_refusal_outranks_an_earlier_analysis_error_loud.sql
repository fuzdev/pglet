-- So is the `ORDER BY` refusal.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: the ORDER BY, not the column (0A000)
-- @expect-message
-- @test
WITH RECURSIVE q AS (SELECT nosuchcol FROM s),
  r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 ORDER BY 1)
SELECT 1;
