-- `ChooseRelationName` is called with `isconstraint = true` for a constraint's index,
-- which also skips any candidate matching an existing constraint name **anywhere in
-- the schema** — stricter than `pg_constraint`'s own per-relation index, because "the
-- SQL standard says that constraint names should be unique within schemas". So a
-- CHECK on an unrelated table can push the auto-name to `uanc2_a_key1`.
-- @setup
CREATE TABLE uanc1 (a INT, CONSTRAINT uanc2_a_key CHECK (a > 0));
CREATE TABLE uanc2 (a INT UNIQUE);

-- @test
SELECT indexname::text AS indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'uanc2' ORDER BY indexname;
