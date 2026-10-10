-- A NOT NULL column with a DEFAULT, omitted at INSERT: the default fills it,
-- so the not-null constraint is satisfied (no violation).
-- @setup
CREATE TABLE nnd (id INT, name TEXT NOT NULL DEFAULT 'anon');
INSERT INTO nnd (id) VALUES (1);

-- @test
SELECT id, name FROM nnd ORDER BY id;
