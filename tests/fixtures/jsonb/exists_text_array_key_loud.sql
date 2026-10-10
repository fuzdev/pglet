-- `?` takes a `text` key, not a `text[]` — that is `?|`/`?&`. The array right operand
-- matches no signature; without the rule it would fall through to a silent NULL.
-- @test
-- @expect-error: no jsonb ? text[] operator
-- @expect-message
SELECT '{"a":1}'::jsonb ? ARRAY['a'];
