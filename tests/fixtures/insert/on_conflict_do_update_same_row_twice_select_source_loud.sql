-- The `INSERT … SELECT` door, over a `UNIQUE` arbiter rather than a primary key: the source
-- produces the key twice, and the second row meets the row the first updated — `21000`.
-- @profile-lock
-- @setup
CREATE TABLE ocss (id INT UNIQUE, a INT, b INT);
INSERT INTO ocss VALUES (1, 10, 0);

-- @test
-- @expect-error: ON CONFLICT DO UPDATE command cannot affect row a second time (21000)
INSERT INTO ocss SELECT v, w, 0 FROM (VALUES (1, 1), (1, 2)) s(v, w)
ON CONFLICT (id) DO UPDATE SET a = ocss.a + excluded.a, b = ocss.a
RETURNING *;
