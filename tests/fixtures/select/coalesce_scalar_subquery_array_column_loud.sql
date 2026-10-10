-- An `ARRAY[…]` over a scalar subquery whose column is certain is a certain `numeric[]`
-- column, so beside a computed `double precision` branch the fold has no common type:
-- `42804`, as on 18.6. The column was not a typed branch, and the fold answered the
-- array with the sibling cast to it.
-- @setup
CREATE TABLE t (f FLOAT8);
INSERT INTO t VALUES (1.5);
-- @expect-error: COALESCE types numeric[] and double precision cannot be matched
-- @test
SELECT COALESCE(q.x, t.f + 0)
FROM t, (SELECT ARRAY[(SELECT avg(x) FROM (VALUES (1), (4)) v(x)), 7] AS x) q;
