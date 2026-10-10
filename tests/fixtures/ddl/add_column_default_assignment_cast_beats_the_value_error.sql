-- **Order, half 2 of 2** (see `default_input_function_beats_the_assignment_cast.sql`).
--
-- `coerce_to_target_type` is a type verdict 18.6 reaches without evaluating the default
-- at all, so an expression that is both ill-typed for the column and arithmetically
-- impossible is the **type** error: `(1/0)::text` into an `INT` column is `42804` there.
-- The `ADD COLUMN` route is the one that can tell the two apart — it is the route that
-- evaluates a default eagerly (`StoreAttrMissingVal`), so before the assignment gate
-- moved ahead of the held value error this statement answered `22012` here.
-- @setup
CREATE TABLE acv (k INT);
-- @expect-error: the assignment cast outranks the eager division by zero (42804)
-- @test
ALTER TABLE acv ADD COLUMN c INT DEFAULT (1/0)::text;
