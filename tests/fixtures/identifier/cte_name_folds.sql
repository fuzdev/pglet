-- A CTE name and the FROM reference to it fold, so `WITH Cte` is reachable as `cte`.
-- The CTE map is keyed by the parsed name, which is a different name position from the
-- catalog's — `parse_with_clause`'s `expect_name` and `parse_table_factor`'s own token
-- read — so it needs its own guard.
-- @test
WITH Cte AS (SELECT 1 AS x) SELECT * FROM cte;
