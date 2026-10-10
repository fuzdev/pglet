-- The other side of the same split: the regclass coercion runs at parse analysis, so a
-- name **no relation at all** holds outranks the argument error that the kind check
-- yields to. 42P01, not 22012.
-- @test
SELECT setval('svmr_no_such_relation', 1/0);
-- @expect-error: the missing relation outranks the argument error (42P01)
