-- The gate is per name, not per `BuiltinFn`: `clock_timestamp` has its own variant and
-- its own arm, and each of the six niladic names must refuse an argument list on its
-- own. 42883 on both engines.
-- @test
-- @expect-error: function clock_timestamp(integer) does not exist
SELECT clock_timestamp(1);
