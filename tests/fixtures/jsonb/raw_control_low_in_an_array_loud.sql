-- `0x01`, the bottom of the refused range, in an array element.
-- @expect-error: a raw 0x01 inside a JSON string
-- @expect-message
-- @test
SELECT (E'["a\x01b"]')::jsonb;
