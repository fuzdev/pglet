-- A symbol outside the base64 alphabet is `22023`.
-- @expect-error: invalid symbol found while decoding base64 sequence
-- @test
SELECT decode('aG!k', 'base64');
