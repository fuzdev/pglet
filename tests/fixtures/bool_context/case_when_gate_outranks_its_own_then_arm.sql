-- The same interleave in `transformCaseExpr`: a `WHEN` condition is coerced
-- before its `THEN` result is transformed at all.
-- @setup
CREATE TABLE bcint4 (a INT);
-- @expect-error: argument of CASE/WHEN must be type boolean, not the THEN arm's 42703
-- @test
SELECT CASE WHEN 1 THEN nosuchcol END FROM bcint4;
