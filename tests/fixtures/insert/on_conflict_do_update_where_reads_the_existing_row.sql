-- A `DO UPDATE … WHERE` reads the existing row, not the row the `SET` list assembles: the
-- predicate holds on the old `a` (10) and the row is updated, although the new `a` (11)
-- would fail it.
-- @setup
CREATE TABLE ocwr (id INT PRIMARY KEY, a INT, b INT);
INSERT INTO ocwr VALUES (1, 10, 0);

-- @test
INSERT INTO ocwr VALUES (1, 50, 60)
ON CONFLICT (id) DO UPDATE SET a = ocwr.a + 1, b = ocwr.a WHERE ocwr.a = 10
RETURNING id, a, b;
