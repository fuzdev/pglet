-- 18.6's `json_lex_string` refuses a byte at or below 31 inside a string
-- (`(unsigned char) *p <= 31`) with the detail `Character with value 0x09 must be
-- escaped.`; pglet used to take it, store it, and write it back out as its escape.
-- The `E''` literal puts the raw byte in the jsonb text; the file holds no raw byte.
-- @expect-error: a raw tab inside a JSON string
-- @expect-message
-- @test
SELECT (E'{"k": "a\tb"}')::jsonb;
