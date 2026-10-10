-- The same statement bound to a value: the comparison decides.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text y
-- @test
SELECT id FROM up_t WHERE (name = $1 OR $1 IS NULL) ORDER BY id;
