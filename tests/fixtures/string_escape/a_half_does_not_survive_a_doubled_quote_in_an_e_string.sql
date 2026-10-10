-- The `U&'...'` doubled-quote guard's `E'...'` twin: `''` is `<xeu>.`, so it refuses a
-- pending half rather than collapsing to a quote the following `\uDCA9` could pair
-- across. Its completing half is what separates a guard that refuses here from one that
-- only notices at the end of the literal.
-- @test
-- @expect-error: a doubled quote cannot sit inside a pair
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT E'\uD83D''\uDCA9';
