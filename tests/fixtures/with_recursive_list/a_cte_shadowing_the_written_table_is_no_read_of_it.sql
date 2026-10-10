-- A CTE named like the table a data-modifying CTE writes shadows it, so a body reading that
-- name reads the CTE and is no read of the write.
-- @setup
CREATE TABLE u (x INT);
INSERT INTO u VALUES (5);
-- @test
WITH RECURSIVE
  r AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM r WHERE n < 2),
  w AS (INSERT INTO u VALUES (6) RETURNING x),
  u AS (SELECT 1 AS x)
SELECT count(*) FROM u, r;
