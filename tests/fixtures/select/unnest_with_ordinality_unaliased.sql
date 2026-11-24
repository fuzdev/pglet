-- Without an alias the item is `unnest` and its columns `unnest` and `ordinality`; the
-- ordinality column is read bare, qualified, and in arithmetic at its `bigint` type.
-- @test
SELECT unnest, ordinality, unnest.ordinality * 2 AS twice FROM unnest(ARRAY['x', 'y']) WITH ORDINALITY ORDER BY 2 DESC;
