-- A data-modifying CTE that reads itself is a recursive query containing a
-- data-modifying statement.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: recursive data-modifying query (42P19)
-- @expect-message
-- @test
WITH RECURSIVE w AS (INSERT INTO s SELECT * FROM w RETURNING x)
SELECT * FROM w;
