-- The message carries the bytes from the failure offset -- `min(pg_utf_mblen(lead),
-- remaining)` of them, so here the two that are left rather than the three
-- `pg_utf_mblen(0xE9)` asks for. The `a` before it is scanned and passed.
-- @test
-- @expect-error: the byte list is clamped to what remains
-- @expect-message
SELECT E'a\xE9b';
