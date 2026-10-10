-- An `"any"` argument after an occurrence that typed the parameter: the comparison
-- earlier in the select list makes `$1` a `text`, and `jsonb_build_object` reads it as
-- one.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
SELECT name = $1 AS m, jsonb_build_object('k', $1) AS j FROM up_t ORDER BY id;
