-- The positive twin of `insert/on_conflict_where_gate_fires.sql`: the DO UPDATE
-- `WHERE` resolves both the target relation and the `EXCLUDED` pseudo-relation, which
-- carries the target's shape — so a bare string literal compared against either side
-- is coerced to the column's type and the gate accepts the pair.
-- @setup
CREATE TABLE oce (id INT PRIMARY KEY, n INT);
INSERT INTO oce VALUES (1, 1), (2, 9);
-- @test
INSERT INTO oce VALUES (1, 5), (2, 5) ON CONFLICT (id) DO UPDATE SET n = EXCLUDED.n WHERE oce.n = '1' AND EXCLUDED.n = '5' RETURNING id, n;
