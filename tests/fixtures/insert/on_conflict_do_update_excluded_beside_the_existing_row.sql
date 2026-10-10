-- A right-hand side reading `excluded` and the existing row: `b = t.a + excluded.b` takes
-- the existing `a` (10), not the `excluded.a` the assignment to its left wrote — `70`.
-- pglet answered `110`.
-- @setup
CREATE TABLE ocex (id INT PRIMARY KEY, a INT, b INT);
INSERT INTO ocex VALUES (1, 10, 0);

-- @test
INSERT INTO ocex VALUES (1, 50, 60)
ON CONFLICT (id) DO UPDATE SET a = excluded.a, b = ocex.a + excluded.b
RETURNING id, a, b;
