-- base64 missing its padding is `22023`.
-- @expect-error: invalid base64 end sequence
-- @test
SELECT decode('aGk', 'base64');
