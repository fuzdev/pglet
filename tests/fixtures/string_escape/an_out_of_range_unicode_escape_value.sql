-- `is_valid_unicode_codepoint` is `0 < c <= 0x10FFFF`; a value outside it is
-- `invalid Unicode escape value` in both decoders.
-- @test
-- @expect-error: a code point above U+10FFFF
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT E'\U00110000';
