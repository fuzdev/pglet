-- The `INSERT` target is a range variable too, so naming the CTE there makes it
-- self-referencing.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: recursive data-modifying query (42P19)
-- @expect-message
-- @test
WITH RECURSIVE w AS (INSERT INTO w VALUES (1) RETURNING x)
SELECT * FROM w;
