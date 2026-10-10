-- A derived table named for a **one-column** base table: `z` binds to the derived
-- table, and the shadowed table — which the query never reads — decides nothing. A
-- reader that resolves the FROM name through `schema.get_table` sees the base table
-- instead, and a check judging the projection against that table's one column would
-- refuse the statement `42703 column "z" does not exist` where it answers over a
-- two-column table.
--
-- That arity is the point of pinning it separately: every other fixture in this group
-- uses a two-column base table. The derived-table / CTE / `unnest` maps claim a FROM name
-- ahead of the catalog, the kind-before-name rule the scope follows.
-- @setup
CREATE TABLE one_col_shadow (a INT);
INSERT INTO one_col_shadow VALUES (1);

-- @test
SELECT z FROM (SELECT 1 AS z) AS one_col_shadow;
