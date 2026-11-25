-- `=` as a group's second symbol is `22023`.
-- @expect-error: unexpected "=" while decoding base64 sequence
-- @test
SELECT decode('a=', 'base64');
