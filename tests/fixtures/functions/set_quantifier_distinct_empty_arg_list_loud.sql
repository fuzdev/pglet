-- `gram.y`'s quantifier productions require a non-empty `func_arg_list`, and no rule
-- combines a quantifier with an empty one, so this is a syntax error at the `)`. pglet's
-- parser ate the keyword and then accepted the empty list, answering a timestamp — the
-- silent half of the grammar bound. 42601 on both.
-- @test
-- @expect-error: syntax error at or near ")"
SELECT now(DISTINCT) IS NOT NULL;
