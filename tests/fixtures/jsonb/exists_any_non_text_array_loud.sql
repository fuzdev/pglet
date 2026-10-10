-- `?|` and `?&` are registered over `text[]` only, so an `integer[]` right operand
-- resolves to no operator; without the rule pglet would answer `f`.
-- @test
-- @expect-error: no jsonb ?| integer[] operator
-- @expect-message
SELECT '{"a":1}'::jsonb ?| ARRAY[1,2];
