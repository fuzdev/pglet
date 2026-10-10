-- The same guard at the doubled-escape arm: `str_udeescape` refuses a pending half
-- there before it emits the backslash, so this is not the two-character `\💩`.
-- The completing `\DCA9` is what makes this the discriminating half of the pair, its
-- weak twin being `a_doubled_escape_does_not_complete_a_pair.sql`.
-- @test
-- @expect-error: a doubled escape char cannot sit inside a pair
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT U&'\D83D\\\DCA9';
