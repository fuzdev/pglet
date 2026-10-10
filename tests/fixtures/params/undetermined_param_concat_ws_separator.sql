-- The neighbour of the `concat_ws` list refusal: the separator position is `text`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text -
-- @test
SELECT concat_ws($1, 'a', 'b') AS c;
