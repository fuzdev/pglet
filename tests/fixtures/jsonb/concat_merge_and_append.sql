-- `jsonb || jsonb`: two objects merge, the right value winning a shared key; anything
-- else appends, an object or a scalar counting as an array of itself.
-- @test
SELECT '{"a": 1}'::jsonb || '{"b": 2, "a": 3}'::jsonb AS merge, '[1]'::jsonb || '[2]'::jsonb AS arrays,
       '1'::jsonb || '[2]'::jsonb AS scalar_array, '{"a": 1}'::jsonb || '[2]'::jsonb AS object_array,
       '1'::jsonb || '2'::jsonb AS scalars, '[]'::jsonb || '{}'::jsonb AS array_object,
       'null'::jsonb || 'null'::jsonb AS nulls, '{"a": 1}'::jsonb || NULL::jsonb AS with_null;
