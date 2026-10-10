-- Control: without `RECURSIVE` a forward reference to a name a base table also has
-- reads the base table.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH q AS (SELECT * FROM s),
  s AS (SELECT 5 AS x)
SELECT x FROM q ORDER BY x;
