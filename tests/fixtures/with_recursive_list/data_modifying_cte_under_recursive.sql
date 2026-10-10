-- A data-modifying CTE under `RECURSIVE` that does not read itself.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE w AS (INSERT INTO s VALUES (7) RETURNING x)
SELECT x FROM w;
