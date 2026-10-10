-- A numeric literal past the format's weight cap is 18.6's `22003 value overflows
-- numeric format`, raised by `numeric_in`'s `make_result` in parse analysis — not a
-- syntax error. Answered as `42601 syntax error: invalid number: <the whole literal>`,
-- the message carries the decoded value and grows with it (180 677 B measured for a
-- 150 000-digit radix literal).
-- @test
-- @expect-error: numeric literal past the format weight cap (22003)
-- @expect-message
SELECT 1e131073;
