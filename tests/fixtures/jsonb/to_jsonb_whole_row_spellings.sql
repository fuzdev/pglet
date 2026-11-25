-- `t.*` is the same whole row, a `WITH` query's row is one too, and a column that shares
-- the FROM item's name is the column, not the row.
-- @setup
CREATE TABLE tj_sp (a INT, b TEXT);
INSERT INTO tj_sp VALUES (1, 'q');
-- @test
WITH w AS (SELECT 7 AS n, 'v' AS s)
SELECT to_jsonb(t.*) AS star, (SELECT to_jsonb(w) FROM w) AS cte,
       (SELECT to_jsonb(c) FROM (SELECT 3 AS c) c) AS column_wins
FROM tj_sp t;
