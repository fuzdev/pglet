-- `unnest(<array>)` as a set-returning function in a no-FROM SELECT projection.
-- Row-expands: one row per array element, with the scalar sibling `'x'` broadcast
-- across them. This is the projection-unnest half of the spine's
-- `query_put_fact_refs` shape (`SELECT $1, unnest($2)`).
-- @test
SELECT 'x' AS s, unnest('{a,b,c}'::text[]) AS u ORDER BY u;
