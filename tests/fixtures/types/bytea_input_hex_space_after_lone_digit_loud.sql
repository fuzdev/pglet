-- @test
-- @expect-error: a space after a lone digit is refused as a hex digit, 22023
SELECT '\x61 6 '::bytea;
