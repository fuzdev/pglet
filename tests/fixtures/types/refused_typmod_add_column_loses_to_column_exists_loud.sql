-- On `ADD COLUMN` the refusal **loses**, and to three gates: 18.6's
-- `check_for_column_name_collision` runs ahead of the `BuildDescForRelation` that resolves
-- the type, so an already-present column answers `42701` whatever the declared type says.
-- @setup
CREATE TABLE rtae (a INT);
-- @expect-error: the existing column outranks the refused length
-- @expect-message
-- @test
ALTER TABLE rtae ADD COLUMN a CHAR(0);
