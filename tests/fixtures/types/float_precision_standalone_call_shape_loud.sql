-- `float (` can begin nothing but a type name — `float` is a `COL_NAME_KEYWORD`, which
-- 18.6's `func_name` excludes — so a `FLOAT(0)` with **no** string literal after it is
-- still the grammar's refusal and not a function call. pglet's speculative read commits
-- on the `(` for the same reason; before that, this shape answered pglet's own
-- `0A000 function not supported: float`.
-- @expect-error: a float precision below 1 bit with no literal following
-- @expect-message
-- @test
SELECT float(0);
