-- The accept side of the `jsonb` unique index: values that differ only in a string's
-- case, in type (`1` and `"1"`), or in nesting are distinct keys, and NULL repeats.
-- @profile-lock
-- @setup
CREATE TABLE cka_j (id INT, j JSONB UNIQUE);
INSERT INTO cka_j (j, id) VALUES ('1', 1), ('"1"', 2), ('[1]', 3), (NULL, 4), (NULL, 5);
INSERT INTO cka_j (j, id) VALUES ('"a"', 6), ('"A"', 7), ('[[1]]', 8), ('{"1":1}', 9), ('[]', 10), ('{}', 11);
-- @test
SELECT id, j::text AS key FROM cka_j ORDER BY id;
