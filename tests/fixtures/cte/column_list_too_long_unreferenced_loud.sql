-- The arity check fires while the `WITH` clause is analysed, so a CTE the body never
-- references still raises. This is why the check lives at registration and not at the
-- reference site.
-- @expect-error: an unreferenced CTE's over-long list still raises
-- @expect-message
-- @test
WITH c(w, x, y, z) AS (SELECT 1, 2, 3) SELECT 1 AS only_column;
