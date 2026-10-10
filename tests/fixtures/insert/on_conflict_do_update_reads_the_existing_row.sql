-- A `DO UPDATE SET` list reads the conflicting row, never an earlier assignment's result:
-- `b = t.a` reads the old `a` (10). pglet answered `11 | 11`.
-- @setup
CREATE TABLE ocer (id INT PRIMARY KEY, a INT, b INT);
INSERT INTO ocer VALUES (1, 10, 0);

-- @test
INSERT INTO ocer VALUES (1, 50, 60)
ON CONFLICT (id) DO UPDATE SET a = ocer.a + 1, b = ocer.a
RETURNING id, a, b;
