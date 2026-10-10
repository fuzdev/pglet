-- The position of the grammar-name rule: `nullif` takes exactly two arguments and this
-- call has one, so an arity check placed ahead of it answers 42883 (which pglet did).
-- On 18.6 the quantifier is a parse error and parsing precedes everything, so the arity
-- is never reached. 42601 on both — the member that fixes which check runs first.
-- @test
-- @expect-error: syntax error at or near "DISTINCT"
SELECT nullif(DISTINCT 1);
