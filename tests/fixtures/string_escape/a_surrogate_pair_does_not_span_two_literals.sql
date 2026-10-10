-- Each literal is finished on its own, so a first half at the end of one and a second
-- at the start of the next do not pair.
-- @test
-- @expect-error: the pair does not cross a literal boundary
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT E'\uD83D' || E'\uDCA9';
