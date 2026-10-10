-- `avg` over an argument whose `integer` type is the walkers' guess — a `COALESCE` the
-- coercion pass could not unify, a branch of it over a scalar subquery whose own column
-- is a guess, read directly and through a derived table — is not the
-- exact `numeric` rule's: it stays `double precision` and averages every value, as 18.6
-- does over the `float8` its own fold resolves to.
-- @setup
CREATE TABLE t (i INT, f FLOAT8);
INSERT INTO t VALUES (1, 1.5), (NULL, 2.5), (3, NULL);
-- @test
SELECT (SELECT avg(COALESCE(i + 0, f + (SELECT * FROM unnest(ARRAY[1])))) FROM t WHERE i IS NOT NULL) AS a1,
  (SELECT avg(x) FROM (SELECT COALESCE(i + 0, f + (SELECT * FROM unnest(ARRAY[1]))) AS x FROM t) d) AS a2;
