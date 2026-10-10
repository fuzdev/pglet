-- `transformReturningClause` runs before the rewriter too, so a `RETURNING` item naming
-- nothing outranks the duplicate. With the `SET` list resolving **last** among an
-- `UPDATE`'s clauses here, this is the boundary the gate had to be placed below.
-- @setup
CREATE TABLE dstg (a INT, b INT);
INSERT INTO dstg VALUES (0, 0);

-- @expect-error: column "nosuch" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dstg SET a = 1, a = 2 RETURNING nosuch;
