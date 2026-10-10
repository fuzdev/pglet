-- What follows a pending first half must be a *second* half; another first is not
-- one.
-- @test
-- @expect-error: two first halves in a row
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT E'\uD83D\uD83D';
