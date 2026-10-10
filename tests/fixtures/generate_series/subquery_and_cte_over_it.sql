-- A derived table, a CTE, an `IN` subquery and a set operation over a series.
-- @setup
CREATE TABLE gs_s (id INT);
INSERT INTO gs_s VALUES (1), (3), (5);
-- @test
WITH s AS (SELECT g FROM generate_series(1, 4) g)
SELECT 'cte' AS k, sum(g) AS v FROM s
UNION ALL SELECT 'derived', sum(d) FROM (SELECT g * 2 AS d FROM generate_series(1, 3) g) t WHERE d > 2
UNION ALL SELECT 'in', count(*) FROM gs_s WHERE id IN (SELECT g FROM generate_series(1, 3) g)
UNION ALL SELECT 'exists', count(*) FROM gs_s WHERE EXISTS (SELECT 1 FROM generate_series(4, 6) g WHERE g = gs_s.id)
ORDER BY 1;
