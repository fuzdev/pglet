-- 18.6's grammar puts a `WITH` list only ahead of a query or a data-modifying statement,
-- so a `TRUNCATE` after one is a syntax error, and nothing is truncated.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: syntax error at TRUNCATE (42601)
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3) TRUNCATE s;
