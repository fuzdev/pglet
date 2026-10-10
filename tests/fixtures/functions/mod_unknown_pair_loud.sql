-- Two string literals fit all four `mod` signatures: 42725.
-- @test
-- @expect-error: function mod(unknown, unknown) is not unique
SELECT mod('7', '3');
