-- The `INSERT` target `s` names the later recursive CTE `s`, which is therefore resolved
-- first; the target itself is the table.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE w AS (INSERT INTO s (x) VALUES (4) RETURNING x),
  s(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM s WHERE x < 2)
SELECT count(*) FROM w;
