-- Arguments that are expressions over constants answer, typed as written.
-- @test
SELECT 'arith' AS k, g FROM generate_series(1, 2 + 1, 1 * 1) g
UNION ALL SELECT 'functions', g FROM generate_series(length('ab'), abs(-4)) g
UNION ALL SELECT 'negated', g FROM generate_series(-2147483647 - 1, -2147483647) g
UNION ALL SELECT 'parenthesized', g FROM generate_series(1, (2)) g
ORDER BY 1, 2;
