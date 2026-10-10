-- The neighbour: the comparison in `WHERE`, the null test in `HAVING`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
SELECT count(*) AS c FROM up_t WHERE name = $1 GROUP BY name HAVING $1 IS NULL;
