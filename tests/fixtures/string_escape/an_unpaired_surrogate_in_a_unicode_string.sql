-- The `U&'...'` decoder keeps the same pairing rule with a saved half rather than a
-- scanner state, and refuses an unfinished one at the end of the literal.
-- @test
-- @expect-error: a first surrogate half with no second
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT U&'\D83D';
