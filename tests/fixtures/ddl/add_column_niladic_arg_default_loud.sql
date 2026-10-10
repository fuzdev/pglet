-- The `ALTER TABLE … ADD COLUMN … DEFAULT` spelling of
-- `niladic_arg_column_default_loud.sql`, over a second builtin name. Same gate, reached
-- through the other DDL route.
-- @setup
CREATE TABLE nb_addcol (a INT);
-- @test
-- @expect-error: function clock_timestamp(integer) does not exist
ALTER TABLE nb_addcol ADD COLUMN b TIMESTAMPTZ DEFAULT (clock_timestamp(1));
