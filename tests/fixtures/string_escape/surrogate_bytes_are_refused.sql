-- A UTF-16 surrogate has no UTF-8 encoding: `pg_utf8_islegal` narrows the `0xED` lead
-- byte's continuation to `0x80..0x9F`.
-- @test
-- @expect-error: the bytes of U+D800 are not UTF-8
-- @expect-message
SELECT E'\xED\xA0\x80';
