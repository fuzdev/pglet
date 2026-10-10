-- A grouped query's column read through a derived table is the base column it passes
-- through, so its type is certain and a computed `float8` sibling beside it in a
-- `COALESCE`/`greatest` unifies the pair at 18.6's `double precision`: `1.5` stays `1.5`,
-- where a column the coercion pass could not vouch for left the set at its written
-- reading and cast the sibling to `integer`.
-- @setup
CREATE TABLE t (k INT, f FLOAT8);
CREATE TABLE u (k INT);
INSERT INTO t VALUES (1, 1.5), (3, NULL);
INSERT INTO u VALUES (1), (3), (5);
-- @test
SELECT t.k, COALESCE(s.x * 1, t.f + 0) AS c, greatest(s.x, t.f + 0) AS g
FROM t JOIN (SELECT k AS x FROM u GROUP BY k) s ON s.x = t.k
ORDER BY 1;
