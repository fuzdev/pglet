-- The jsonb arm, with an **even** argument list so its parity check passes and the
-- quantifier is the only thing wrong. Its odd-list twin
-- (`set_quantifier_distinct_beats_jsonb_parity_loud.sql`) pins the
-- precedence between the two. 42809 on both; pglet answered `{"a": 1}`.
-- @test
-- @expect-error: DISTINCT specified, but jsonb_build_object is not an aggregate function
SELECT jsonb_build_object(DISTINCT 'a', 1);
