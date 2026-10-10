-- A FROM-clause `unnest` of an array a cast converts yields elements of the cast's
-- element type, so its column is certain: `avg` over it is the exact `numeric` and `sum`
-- of a `bigint` one `numeric`, past the `bigint` range.
-- @test
SELECT (SELECT avg(x) FROM unnest('{1,4}'::int[]) AS u(x)) AS a,
  (SELECT round(avg(x)) FROM unnest(ARRAY[1, 4]::smallint[]) AS u(x)) AS r,
  (SELECT sum(x) FROM unnest('{9223372036854775807,1}'::bigint[]) AS u(x)) AS s,
  (SELECT avg(x) FROM unnest(ARRAY[NULL, '7']::int[]) AS u(x)) AS n;
