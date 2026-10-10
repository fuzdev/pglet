-- The neighbour: the cast is the earlier argument.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
SELECT jsonb_build_object('k', $1::text, 'j', $1) AS j;
