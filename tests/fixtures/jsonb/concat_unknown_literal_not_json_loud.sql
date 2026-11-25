-- ...so a literal that is no JSON is `jsonb`'s `22P02`.
-- @expect-error: invalid input syntax for type json
-- @test
SELECT '{"a": 1}'::jsonb || 'x';
