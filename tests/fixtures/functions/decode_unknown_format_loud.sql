-- A format `decode` does not have is `22023`.
-- @expect-error: unrecognized encoding
-- @test
SELECT decode('x', 'nope');
