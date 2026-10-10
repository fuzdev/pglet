-- The function is strict: any NULL argument, typed or not, is zero rows — ahead of a
-- zero step's error.
-- @test
SELECT 'bare' AS k, g FROM generate_series(1, NULL) g
UNION ALL SELECT 'typed', g FROM generate_series(NULL::int, 3) g
UNION ALL SELECT 'step', g FROM generate_series(1, 3, NULL) g
UNION ALL SELECT 'before_zero_step', g FROM generate_series(NULL::int, 3, 0) g
UNION ALL SELECT 'bigint', g FROM generate_series(1, NULL::bigint) g
ORDER BY 1, 2;
