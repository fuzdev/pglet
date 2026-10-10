-- substr: substring's evaluation under its own name — text in characters, bytea in
-- bytes, the bytea overload answering bytea; NULL in any argument is NULL; a
-- string-literal start or length resolves to int4.
-- @setup
CREATE TABLE sb (id INT, t TEXT, b BYTEA, st INT, ln INT);
INSERT INTO sb VALUES
  (1, 'abcdef', '\x616263646566', 2, 3),
  (2, 'héllo', '\x00ff0001', -1, 3),
  (3, 'xyz', '\x', 1, 0),
  (4, NULL, NULL, 1, 1),
  (5, 'abc', '\x61', NULL, 1),
  (6, 'abc', '\x61', 1, NULL);
-- @test
SELECT id,
  substr(t, st) AS t2, substr(t, st, ln) AS t3,
  substr(b, st) AS b2, substr(b, st, ln) AS b3,
  substr(t, '2', '2') AS t_unknown
FROM sb ORDER BY id;
