-- A `GROUP BY` ordinal under `SELECT *` counts against the **expanded** select list, so
-- `1, 2` are the table's two columns. This needs the star expanded twice over: to count
-- the ordinal against, and because an aggregated query cannot carry the unexpanded bare
-- `*` through to the executor — `SELECT * FROM t GROUP BY x, y` (no ordinal involved)
-- was rejected as `42703 column "*" does not exist` on the same path.
-- @setup
CREATE TABLE gbs (x INT, y INT);
INSERT INTO gbs VALUES (1, 10), (1, 20), (2, 30);
-- @test
SELECT * FROM gbs GROUP BY 1, 2 ORDER BY 1, 2;
