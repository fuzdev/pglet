-- A first surrogate half with nothing to complete it is refused rather than emitted:
-- 18.6's scanner is left in the `xeu` state and the closing quote is an error there.
-- @test
-- @expect-error: a first surrogate half with no second
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT E'\uD83D';
