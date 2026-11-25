-- An odd number of hex digits is `22023`.
-- @expect-error: invalid hexadecimal data: odd number of digits
-- @test
SELECT decode('abc', 'hex');
