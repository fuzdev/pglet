-- The INTERNAL edge is raised in `findDependentObjects` at the outermost recursion
-- level with no `DropBehavior` test anywhere in the path, so `CASCADE` does **not**
-- help — the one place it separates the identity spelling from the `SERIAL` one,
-- whose `pg_attrdef` edge cascades away.
-- @setup
CREATE TABLE dsidc_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
DROP SEQUENCE dsidc_t_id_seq CASCADE;
-- @expect-error: an identity column requires its sequence, CASCADE notwithstanding (2BP01)
