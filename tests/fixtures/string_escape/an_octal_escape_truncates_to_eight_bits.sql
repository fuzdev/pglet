-- `{xeoctesc}` takes up to three octal digits and `strtoul`'s result lands in an
-- `unsigned char`, so `\777` is the byte `0xFF` -- a lone high byte, and an error.
-- Without the truncation it would be the code point U+01FF and no error at all.
-- @test
-- @expect-error: an octal escape above 0xFF truncates
-- @expect-message
SELECT E'\777';
