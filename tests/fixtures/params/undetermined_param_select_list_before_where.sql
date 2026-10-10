-- Clause order: the select list is analysed before `WHERE`, so a comparison there
-- types the parameter ahead of the null test.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT id, name = $1 AS m FROM up_t WHERE $1 IS NULL ORDER BY id;
