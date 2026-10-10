-- `RemoveRelations` decides the relkind on a name that resolves, so a table named
-- here is the wrong object type rather than a missing sequence.
-- @setup
CREATE TABLE dsot_t (a INT);
-- @test
DROP SEQUENCE dsot_t;
-- @expect-error: a table is not a sequence (42809)
