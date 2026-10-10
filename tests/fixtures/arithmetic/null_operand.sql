-- A NULL arithmetic operand yields NULL (the old f64 path treated NULL as 0,
-- so `NULL + 1` wrongly returned 1).
-- @test
SELECT NULL::int4 + 1 AS r;
