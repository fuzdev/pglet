-- The `U&'...'` half of the split above: the grammar's own unescaper raises a plain
-- `42601` for the same malformation the `E'...'` scanner answers `22025` for. 18.6
-- disagrees with itself across its two decoders and both sides are reproduced.
-- @test
-- @expect-error: a short Unicode escape in a U& string is 42601
-- @no-error-fields: pglet's `P` is the string literal's own start where 18.6's is the
-- offending escape inside it (and, for an unpaired first half, the token the `xeu`
-- state fails at). That granularity is the lexer's everywhere, not this rule's.
SELECT U&'\00';
