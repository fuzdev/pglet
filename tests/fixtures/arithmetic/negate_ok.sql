-- A non-MIN negate is unchanged across the integer widths — only the MIN value
-- (which has no positive counterpart) fails loud.
-- @test
SELECT -('-32767'::int2) AS a, -(2147483647) AS b, -(5::int8) AS c;
