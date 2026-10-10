-- A body reading a table no data-modifying CTE writes answers.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
CREATE TABLE u (y INT);
-- @test
WITH RECURSIVE w AS (INSERT INTO u VALUES (9) RETURNING y) SELECT count(*) FROM s;
