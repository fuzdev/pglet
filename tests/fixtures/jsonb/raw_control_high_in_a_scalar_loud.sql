-- `0x1F`, the top of the refused range (`<= 31`), in a top-level scalar string.
-- @expect-error: a raw 0x1F inside a JSON string
-- @expect-message
-- @test
SELECT (E'"a\x1fb"')::jsonb;
