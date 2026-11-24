-- A no-FROM aggregate in a scalar subquery: the subquery's own one row.
-- @test
SELECT 1 AS one WHERE (SELECT count(*)) = 1;
