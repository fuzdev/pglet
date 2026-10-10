-- The negative ordinal is refused at **every** query level, not only the outermost. A
-- derived table resets the correlation depth and lowers its own scope, so its `ORDER BY`
-- reaches the ordinal check through a different path than the top-level clause — and it
-- was silent there too. 18.6: `42P10`, with the position reported as `-1`.
-- @setup
CREATE TABLE obdt (x INT, y INT);
INSERT INTO obdt VALUES (3, 1), (1, 2), (2, 3);
-- @test
-- @expect-error: ORDER BY position -1 is not in select list (42P10)
SELECT d.x FROM (SELECT x FROM obdt ORDER BY -1) d;
