-- A subquery's qualified reference to the series' column, under the alias and under a
-- column list, binds the outer row's value.
-- @setup
CREATE TABLE gs_cq (n INT);
INSERT INTO gs_cq VALUES (1), (2), (2);
-- @test
SELECT 'scalar' AS k, g AS v, (SELECT count(*) FROM gs_cq WHERE gs_cq.n = g.g) AS c FROM generate_series(1, 3) g
UNION ALL SELECT 'exists', i, 1 FROM generate_series(1, 3) AS g(i) WHERE EXISTS (SELECT 1 FROM gs_cq WHERE gs_cq.n = g.i)
ORDER BY 1, 2;
