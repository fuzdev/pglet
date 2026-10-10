-- An aggregate under the value constructors and variadic functions: `greatest`, `least`,
-- `nullif`, `IN`, `ARRAY[…]`, `= ANY`, a searched `CASE` and a nested `COALESCE`. The
-- variadics unify an aggregate argument as its siblings by type, so `least(sum(a), 1)` is
-- `bigint`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT greatest(max(a), min(b)) AS g, least(sum(a), 1) AS l, nullif(count(*), 2) AS n,
  max(a) IN (1, 2) AS i, ARRAY[max(a), min(a)] AS arr, max(a) = ANY (ARRAY[2]) AS an,
  CASE WHEN count(*) > 1 THEN max(a) ELSE min(a) END AS c,
  coalesce(nullif(max(a), 2), 0) AS co
FROM u;
