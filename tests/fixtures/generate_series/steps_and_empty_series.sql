-- A negative step counts down; a series whose start is past its stop in the step's
-- direction is empty; a step that overshoots the stop stops before it.
-- @test
SELECT 'down' AS k, g FROM generate_series(3, 1, -1) g
UNION ALL SELECT 'stride', g FROM generate_series(1, 10, 4) g
UNION ALL SELECT 'stride_down', g FROM generate_series(10, 1, -4) g
UNION ALL SELECT 'one', g FROM generate_series(5, 5, -1) g
UNION ALL SELECT 'start_past_stop', g FROM generate_series(3, 1) g
UNION ALL SELECT 'wrong_direction', g FROM generate_series(1, 3, -1) g
UNION ALL SELECT 'min_step', g FROM generate_series(1, 3, -2147483648) g
UNION ALL SELECT 'negatives', g FROM generate_series(-3, 3, 3) g
ORDER BY 1, 2;
