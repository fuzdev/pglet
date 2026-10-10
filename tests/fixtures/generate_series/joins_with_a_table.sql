-- Joined with a table: inner, left, right and cross.
-- @setup
CREATE TABLE gs_j (id INT, name TEXT);
INSERT INTO gs_j VALUES (1, 'a'), (2, 'b'), (4, 'd');
-- @test
SELECT 'inner' AS k, t.id, g FROM gs_j t JOIN generate_series(1, 3) g ON t.id = g
UNION ALL SELECT 'left', t.id, g FROM gs_j t LEFT JOIN generate_series(1, 3) g ON t.id = g
UNION ALL SELECT 'right', t.id, g FROM gs_j t RIGHT JOIN generate_series(1, 3) g ON t.id = g
UNION ALL SELECT 'cross', t.id, g FROM gs_j t, generate_series(1, 2) g
UNION ALL SELECT 'series_left', t.id, g FROM generate_series(1, 2) g, gs_j t WHERE t.id = g
ORDER BY 1, 2, 3;
