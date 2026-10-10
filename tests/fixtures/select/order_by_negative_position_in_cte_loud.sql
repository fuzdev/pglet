-- The CTE spelling of `order_by_negative_position_in_derived_table_loud.sql`: a `WITH`
-- body is a query level of its own and was equally silent. 18.6: `42P10`.
-- @setup
CREATE TABLE obct (x INT, y INT);
INSERT INTO obct VALUES (3, 1), (1, 2), (2, 3);
-- @test
-- @expect-error: ORDER BY position -1 is not in select list (42P10)
WITH c AS (SELECT x FROM obct ORDER BY -1) SELECT x FROM c;
