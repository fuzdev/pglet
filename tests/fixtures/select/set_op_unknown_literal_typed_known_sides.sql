-- The known side's type is taken whatever it is: an array, a `numeric`, a `boolean`.
-- @test
SELECT ARRAY[1] AS a, 1.5 AS n, true AS b UNION ALL SELECT '{2,3}', '2.25', 'f' ORDER BY 2;
