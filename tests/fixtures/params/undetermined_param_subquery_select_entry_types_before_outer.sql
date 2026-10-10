-- A bare select-list `$1` in a **subquery** is `text` once that subquery is analysed —
-- ahead of what the outer level analyses next.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT (SELECT $1) AS p, $1 IS NULL AS b;
