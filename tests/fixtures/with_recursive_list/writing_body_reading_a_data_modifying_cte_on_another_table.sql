-- A writing body that reads a data-modifying CTE on another table answers as 18.6 does:
-- the CTE's rows reach the body, and neither write sees the other's table.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
CREATE TABLE u (y INT);
-- @test
WITH w AS (INSERT INTO u VALUES (9) RETURNING y) INSERT INTO s SELECT 100 + y FROM w RETURNING x;
