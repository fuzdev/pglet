-- The other side of the same clamp: four bad bytes remain and `pg_utf_mblen(0xE9)` is
-- 3, so three are named.
-- @test
-- @expect-error: the byte list is clamped to the lead byte length
-- @expect-message
SELECT E'\xE9\xE9\xE9\xE9';
