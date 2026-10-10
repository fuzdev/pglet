-- `ADD COLUMN` names its PK/UNIQUE **index first**, then its CHECKs — the opposite of
-- `CREATE TABLE`, and not derivable from it. Postgres's `ALTER TABLE` passes run
-- ascending and `AT_PASS_ADD_CONSTR` (6) is *ahead* of `AT_PASS_ADD_INDEX` (9); what
-- inverts the observable order is that nothing final happens in pass 6 — the inline
-- `UNIQUE` is scheduled straight into pass 9 as an `AT_AddIndex`, while the CHECK is
-- an `AT_AddConstraint` that pass 6 only *re-files* into `AT_PASS_ADD_OTHERCONSTR`
-- (10) via `ATParseTransformCmd`. `DefineRelation` instead attaches the CHECKs as the
-- heap is created and builds the indexes after. Here the declared `UNIQUE` takes
-- `ainbc_c_check` and the CHECK yields to it with `ainbc_c_check1`; written as a
-- `CREATE TABLE` the same column definition is a 42710 instead (see
-- `create_table_check_named_before_index.sql`).
-- @setup
CREATE TABLE ainbc (a INT);
ALTER TABLE ainbc ADD COLUMN c INT CONSTRAINT ainbc_c_check UNIQUE CHECK (c > 0);

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'ainbc'
ORDER BY constraint_name;
