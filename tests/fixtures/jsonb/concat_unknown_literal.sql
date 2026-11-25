-- A bare string literal beside a `jsonb` operand reads as `jsonb`, on either side.
-- @test
SELECT '{"a": 1}'::jsonb || '{"b": 2}' AS right_lit, '{"b": 2}' || '{"a": 1}'::jsonb AS left_lit;
