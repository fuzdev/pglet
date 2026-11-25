-- A typed `text` operand makes it text concatenation, as on 18.6.
-- @test
SELECT '{"a": 1}'::jsonb || 'x'::text AS t;
