-- The same `42702` on the `SET` **right-hand side**, which is the more surprising leg:
-- `DO UPDATE SET n = n + 1` reads like the canonical increment idiom, but `excluded`
-- is in scope there too, so Postgres rejects the bare `n` and the spelling has to be
-- `ocs.n + 1`. pglet used to accept it and bind to the target — the right answer by
-- luck, from a resolution rule that is wrong wherever the two rows differ.
--
-- The `SET` target on the *left* is a bare column name in both engines: it is the
-- column being assigned, not a reference to a row, so it is never ambiguous.
-- @setup
CREATE TABLE ocs (id INT PRIMARY KEY, n INT);
INSERT INTO ocs VALUES (1, 40);
-- @expect-error: bare column reference is ambiguous under ON CONFLICT (42702)
-- @no-profile-lock: the error is raised while the ON CONFLICT action is lowered, before any index selection runs
-- @test
INSERT INTO ocs VALUES (1, 50) ON CONFLICT (id) DO UPDATE SET n = n + 1;
