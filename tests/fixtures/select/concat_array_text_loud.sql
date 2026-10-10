-- `anynonarray` explicitly excludes arrays, so `int[] || text` matches no `||`
-- signature at all (the array forms need a matching element type). pglet used to
-- stringify the array and return '{1,2}x'.
-- @test
-- @expect-error: no integer[] || text operator
SELECT ARRAY[1,2] || 'x'::text;
