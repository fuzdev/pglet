-- `FROM` is analysed before the select list.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
SELECT $1 IS NULL AS b FROM (SELECT id FROM up_t WHERE name = $1) s;
