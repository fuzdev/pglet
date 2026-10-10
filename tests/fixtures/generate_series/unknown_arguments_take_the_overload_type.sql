-- A bare string literal takes the resolved overload's type and is read as it, as 18.6's
-- input function reads it — surrounding spaces included.
-- @test
SELECT 'stop' AS k, g FROM generate_series(1, '3') g
UNION ALL SELECT 'start', g FROM generate_series('1', 3) g
UNION ALL SELECT 'step', g FROM generate_series(1, 3, '2') g
UNION ALL SELECT 'spaced', g FROM generate_series(1, ' 2 ') g
ORDER BY 1, 2;
