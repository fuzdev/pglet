-- A bare `RETURNING` entry is `text` once the `RETURNING` list is analysed, which is
-- before the `SET` list: the null test in the assignment reads a typed parameter.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
UPDATE up_t SET name = CASE WHEN $1 IS NULL THEN 'a' ELSE 'b' END WHERE id = 1 RETURNING $1 AS p, name;
