-- The typed-constant position takes `numeric`'s refusals too — one resolver under two
-- entries, the `ConstTypename` one differing only in the parenless `char` default, which
-- no `numeric` spelling can see.
-- @expect-error: a NUMERIC precision below 1 at a typed constant
-- @expect-message
-- @test
SELECT numeric(0) '1.5' AS never_reached;
