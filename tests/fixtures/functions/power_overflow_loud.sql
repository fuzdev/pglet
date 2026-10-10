-- A float8 power past the range: 22003.
-- @test
-- @expect-error: value out of range: overflow
SELECT power(10, 400);
