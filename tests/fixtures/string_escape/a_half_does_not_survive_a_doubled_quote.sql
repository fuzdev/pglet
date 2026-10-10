-- And at the doubled-quote arm, which the scanner has already collapsed to one `'` by
-- the time the unescaper reads it.
-- @test
-- @expect-error: a doubled quote cannot sit inside a pair
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT U&'\D83D''\DCA9';
