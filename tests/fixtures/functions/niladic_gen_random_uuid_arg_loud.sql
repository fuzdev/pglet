-- The UUID half of the niladic-builtin class — a different `BuiltinFn` and a different
-- result type, the same refusal. 42883 on both engines.
-- @test
-- @expect-error: function gen_random_uuid(integer) does not exist
SELECT gen_random_uuid(1);
