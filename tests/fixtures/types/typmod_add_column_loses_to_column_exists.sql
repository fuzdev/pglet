-- `ADD COLUMN` reads the relation and the column name **before** the declared type,
-- which is where 18.6 puts it (`check_for_column_name_collision` runs ahead of the
-- `BuildDescForRelation` that resolves the type). So an already-present column outranks
-- the modifier refusal rather than the other way round.
-- @setup
CREATE TABLE tmord (a INT);
-- @expect-error: the column already exists, which outranks the type modifier
-- @expect-message
-- @test
ALTER TABLE tmord ADD COLUMN a int4(3);
