-- Whitespace is skipped only between pairs: inside one it is an invalid digit (`22023`).
-- @expect-error: invalid hexadecimal digit
-- @test
SELECT decode('a bcd', 'hex');
