-- numeric → integer casts round half-away-from-zero (Postgres numeric rounding).
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);
-- @test
SELECT 0.5::numeric::int AS a, 1.5::numeric::int AS b, 2.5::numeric::int AS c,
       (-2.5)::numeric::int AS d, (-0.5)::numeric::int AS e
FROM t;
