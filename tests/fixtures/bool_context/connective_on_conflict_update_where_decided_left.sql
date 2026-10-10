-- An `ON CONFLICT DO UPDATE … WHERE` whose left operand decides it: `false` skips the
-- update, `true` takes it, and neither evaluates the division.
-- @setup
CREATE TABLE sq (id INT PRIMARY KEY, n INT);
INSERT INTO sq VALUES (1, 0);
INSERT INTO sq VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET n = excluded.n
  WHERE (sq.id = 99 AND 1/(sq.id-1) = 0);
-- @test
INSERT INTO sq VALUES (1, 6) ON CONFLICT (id) DO UPDATE SET n = excluded.n
  WHERE (sq.id = 1 OR 1/(sq.id-1) = 0) RETURNING id, n;
