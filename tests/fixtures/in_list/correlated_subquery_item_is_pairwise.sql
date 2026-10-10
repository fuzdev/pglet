-- A subquery naming the list's own level is no constant: it is compared on its own, and the
-- one constant beside it is no array, so the stored `real` `0.1` matches neither.
-- @setup
CREATE TABLE inl (id INT, x REAL, d FLOAT8, n NUMERIC, i INT, b BIGINT, s SMALLINT);
INSERT INTO inl VALUES
  (1, 0.1, 0.1, 0.1, 1, 1, 1),
  (2, 1.5, 1.5, 1.5, 2, 2, 2),
  (3, 16777216, 9007199254740992, 16777217, 16777217, 9007199254740993, 3),
  (4, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT o.id FROM inl o
WHERE o.x IN ((SELECT q.n FROM inl q WHERE q.id = o.id)::numeric, 7) ORDER BY o.id;
