-- One `bigint` argument resolves the `bigint` overload, and the column is `bigint`
-- whichever argument it is — a literal past `integer` included.
-- @test
SELECT 'start' AS k, g FROM generate_series(1::bigint, 3) g
UNION ALL SELECT 'step', g FROM generate_series(1, 3, 1::bigint) g
UNION ALL SELECT 'literal', g FROM generate_series(2147483647, 2147483648) g
UNION ALL SELECT 'unknown', g FROM generate_series(1::bigint, '2') g
ORDER BY 1, 2;
