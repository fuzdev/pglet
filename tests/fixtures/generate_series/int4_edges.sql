-- At the edges of `integer` a series ends rather than wrapping: the value past the last
-- one would overflow, and is never emitted.
-- @test
SELECT 'top' AS k, g FROM generate_series(2147483646, 2147483647) g
UNION ALL SELECT 'stride', g FROM generate_series(-2147483648, 2147483647, 2147483647) g
UNION ALL SELECT 'bottom', g FROM generate_series(-2147483647, -2147483648, -1) g
UNION ALL SELECT 'top_down', g FROM generate_series(2147483647, 2147483646, -2147483648) g
ORDER BY 1, 2;
