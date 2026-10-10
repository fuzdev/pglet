-- An `ON CONFLICT DO UPDATE … WHERE` sees **both** the target relation and the
-- `excluded` pseudo-relation, and `excluded` carries the target's whole shape — so a
-- bare `n` names a column two relations claim and Postgres raises `42702`. pglet used
-- to accept it and silently bind to the target row, which here means the `WHERE` reads
-- the *stored* 40 rather than the incoming 50 and the upsert quietly does nothing.
-- Silently taking one of two readings is the shape this engine fails loud on.
-- @setup
CREATE TABLE ocb (id INT PRIMARY KEY, n INT);
INSERT INTO ocb VALUES (1, 40);
-- @expect-error: bare column reference is ambiguous under ON CONFLICT (42702)
-- @no-profile-lock: the error is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO ocb VALUES (1, 50) ON CONFLICT (id) DO UPDATE SET n = EXCLUDED.n WHERE n > 45;
