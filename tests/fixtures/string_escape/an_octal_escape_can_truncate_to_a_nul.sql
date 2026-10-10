-- The same truncation landing on zero: `\400` is the byte `0x00`, which the encoding
-- check refuses exactly as a written `\0` is refused.
-- @test
-- @expect-error: an octal escape that truncates to a NUL
-- @expect-message
SELECT E'\400';
