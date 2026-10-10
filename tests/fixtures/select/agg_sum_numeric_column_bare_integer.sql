-- A `numeric`-declared column some of whose rows pglet leaves as unconverted integers
-- (a `CASE` branch over a function-scan subquery the coercion pass cannot type): the
-- exact `numeric` accumulator reads each integer exactly — 18.6's `integer → numeric`
-- conversion — rather than skipping it, so the group sums are 18.6's `2.0` and `4`.
-- The column itself is not projected: its unconverted rows are sent in an integer
-- binary encoding (`EDGES.md` row 152).
-- @test
SELECT count(*) AS c, sum(DISTINCT n) AS sd, sum(n) AS s, avg(n) AS a
FROM (SELECT CASE WHEN x = 7 THEN (SELECT * FROM unnest(ARRAY[1.0::numeric])) ELSE x END AS n
      FROM (VALUES (1), (7), (2), (2)) v(x)) s
GROUP BY n ORDER BY n;
