-- `tags text[] UNIQUE`: an element boundary is part of the key (`{a,b}` is not `{ab}`),
-- an empty string is an element and neither a NULL element nor the end of the array,
-- and a NULL key repeats.
-- @profile-lock
-- @setup
CREATE TABLE cka_t (id INT, tags TEXT[] UNIQUE);
INSERT INTO cka_t (tags, id) VALUES ('{a,b}', 1), ('{ab}', 2), ('{}', 3), ('{""}', 4), (NULL, 5), (NULL, 6), ('{a,NULL}', 7), ('{"",""}', 8), ('{NULL}', 9), ('{a}', 10), ('{b,a}', 11);
-- @test
SELECT id, tags FROM cka_t ORDER BY id;
