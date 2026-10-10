-- A two-byte lead followed by an ASCII byte is not a sequence, and both bytes are
-- named because `pg_utf_mblen(0xC3)` is 2 and two remain.
-- @test
-- @expect-error: a continuation byte that is not one
-- @expect-message
SELECT E'\xC3\x41';
