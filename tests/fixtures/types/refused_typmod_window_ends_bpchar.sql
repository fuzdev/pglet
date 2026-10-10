-- The `char` half of the same polarity, plus the two spellings that pin the number's
-- **second** read: the length now goes through the `Iconst` the grammar proves and then
-- through the type that validates it, so the lexer's decoded payload (`0x3`) and a
-- leading zero (`03`) both have to arrive as the length `3`.
-- @no-declared-types: a `char(n)` column declares 1043 (varchar) here against 18.6's 1042
--   (bpchar) — pglet keeps no char-vs-varchar bit, the standing gap `EDGES.md` row 95
--   records, and it is outside this fixture's subject.
-- @test
SELECT 'ab'::char(1) AS cmin,
       'abcd'::char(0x3) AS radix,
       'abcd'::char(03) AS leading_zero;
