-- The `bigint` overload's edges, in both directions: the series ends rather than wrapping.
-- @test
SELECT 'top' AS k, g FROM generate_series(9223372036854775806, 9223372036854775807) g
UNION ALL SELECT 'stride', g FROM generate_series(-9223372036854775807, 9223372036854775807, 9223372036854775807) g
UNION ALL SELECT 'down', g FROM generate_series(9223372036854775807, 9223372036854775806, -9223372036854775807) g
ORDER BY 1, 2;
