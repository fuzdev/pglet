-- A variadic over an aggregate and a literal takes the common type of both, as 18.6 does:
-- the aggregate's own type folds into the target rather than being cast to the literal's
-- (`least(sum(i), 5)` is `bigint`, `greatest(max(i), 2.5)` `numeric`).
-- @setup
CREATE TABLE r (i INT, f FLOAT8);
INSERT INTO r VALUES (7, 1.5), (3, 2.5);
-- @test
SELECT least(sum(i), 5) AS a, greatest(max(i), 2.5) AS b, nullif(sum(i), 10) AS c,
  nullif(max(i), 7::bigint) AS d, greatest(avg(i), 1) AS e, least(max(f), 2) AS g,
  greatest(count(*), max(i)) AS h
FROM r;
