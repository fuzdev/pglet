-- The accept side of the same rule, so the gate cannot be made vacuous by refusing the
-- family outright: every spelling over the operands Postgres registers still answers.
-- @test
SELECT '{"a":1,"b":2}'::jsonb ? 'a' AS k,
       '{"a":1}'::jsonb ?| ARRAY['b','a'] AS any_k,
       '{"a":1,"b":2}'::jsonb ?& ARRAY['a','b'] AS all_k,
       '{"a":{"b":3}}'::jsonb #> ARRAY['a','b'] AS path,
       '{"a":{"b":3}}'::jsonb #>> ARRAY['a','b'] AS path_text;
