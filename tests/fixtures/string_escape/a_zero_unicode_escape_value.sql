-- The low end of the same gate. A NUL written as a *Unicode* escape is this syntax
-- error, where a NUL written as a *byte* escape is the encoding error -- the two
-- spellings of one character answer different codes on both engines.
-- @test
-- @expect-error: U+0000 is not a valid code point
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT U&'\0000';
