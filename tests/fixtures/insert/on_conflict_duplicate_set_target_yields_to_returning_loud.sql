-- Being a rewriter check it runs after every clause of the statement is transformed, so a
-- `RETURNING` item naming nothing outranks it. That is the boundary the gate had to be
-- placed below: run where the `ON CONFLICT` clause is lowered — above `RETURNING` — it
-- would answer `42601` here.
-- @setup
CREATE TABLE ocdr (a INT PRIMARY KEY, b INT);
INSERT INTO ocdr VALUES (1, 1);

-- @expect-error: column "nosuch" does not exist (42703)
-- @profile-lock
-- @test
INSERT INTO ocdr VALUES (1, 9) ON CONFLICT (a) DO UPDATE SET b = 1, b = 2 RETURNING nosuch;
