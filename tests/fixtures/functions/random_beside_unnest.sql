-- A volatile call beside a projection's `unnest(…)` is evaluated for each row the
-- `unnest` emits, as 18.6's ProjectSet evaluates it; a stable one answers once.
-- @test
SELECT count(DISTINCT r) AS draws, count(DISTINCT g) AS uuids, count(DISTINCT n) AS nows
FROM (SELECT unnest(ARRAY[1, 2, 3, 4]) AS u, random() AS r, gen_random_uuid() AS g, now() AS n) s;
