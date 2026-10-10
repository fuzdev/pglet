-- The same knowable-array-literal rule outside `ARRAY[…]` itself, and on an **empty**
-- table so nothing evaluates: `COALESCE(integer, integer[])` is a category mismatch
-- Postgres reports at parse time, where pglet returned zero rows and no error.
-- @setup
CREATE TABLE cab (i4 INT4);
-- @expect-error: COALESCE types integer and integer[] cannot be matched (42804)
-- @test
SELECT COALESCE(i4, ARRAY[1]) FROM cab;
