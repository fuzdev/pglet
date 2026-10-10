-- A body whose columns come from a `VALUES` relation rather than a base table: the
-- rename is a property of the CTE relation, not of what the body reads. (A **bare**
-- `VALUES` CTE body is a separate, pre-existing parser gap — `0A000` as a statement,
-- `42601` here — so the relation is reached through a derived table.)
-- @test
WITH c(v, w) AS (SELECT * FROM (VALUES (1, 'a'), (2, 'b')) AS k) SELECT v, w FROM c ORDER BY v;
