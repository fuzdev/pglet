-- The CTE spelling of `select/one_column_table_shadowed_by_a_derived_table.sql` — a
-- CTE named for a one-column base table whose column the query does not name, reached
-- through `Parser::ctes` rather than `parse_derived_table`. The two routes build the
-- FROM item's `(name, alias)` pair in different places, so each is pinned.
--
-- The qualified spelling (`SELECT one_col_cte.z FROM …`) answers on both engines too,
-- and is left unpinned here.
-- @setup
CREATE TABLE one_col_cte (a INT);
INSERT INTO one_col_cte VALUES (1);

-- @test
WITH one_col_cte AS (SELECT 1 AS z) SELECT z FROM one_col_cte;
