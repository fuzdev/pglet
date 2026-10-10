-- The restore half: a postfix sitting *after* a parenthesized group is back at the
-- `b_expr` level, so `(now()) AT TIME ZONE 'utc'` is a syntax error even though
-- `(now() AT TIME ZONE 'utc')` is accepted. Measured on 18.6.
-- @expect-error: the postfix is outside the parens, so b_expr applies again
-- @test
CREATE TABLE def_ppost (a INT, b TIMESTAMP DEFAULT (now()) AT TIME ZONE 'utc');
