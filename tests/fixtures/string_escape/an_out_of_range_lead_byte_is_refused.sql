-- `0xF5` would start a code point above U+10FFFF, which `pg_utf8_islegal` refuses on
-- the lead byte (`a > 0xF4`).
-- @test
-- @expect-error: a lead byte above U+10FFFF
-- @expect-message
SELECT E'\xF5\x80\x80\x80';
