-- `pg_utf8_islegal` refuses an overlong encoding (`a >= 0x80 && a < 0xC2` on the lead
-- byte), and so does Rust's own UTF-8 acceptance set.
-- @test
-- @expect-error: an overlong encoding of U+0000
-- @expect-message
SELECT E'\xC0\x80';
