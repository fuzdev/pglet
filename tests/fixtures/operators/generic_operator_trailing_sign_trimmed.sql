-- …and goes where it does not: `1 =- 1` is `1 = -1`, and `1 <>-1` is `1 <> -1`.
-- @test
SELECT 1 =- 1 AS eq, 1 <>-1 AS ne, 1*-1 AS mul, 1+-+-1 AS signs;
