-- A `UNION` body is renamed too — the list applies to the relation, not to a
-- projection, so a body shape with no target list of its own is no obstacle.
-- @test
WITH c(v) AS (SELECT 1 UNION ALL SELECT 2) SELECT v FROM c ORDER BY v;
