-- Postgres numeric input forms: leading dot, trailing dot, exponent, whitespace.
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);
-- @test
SELECT '.5'::numeric AS a, '5.'::numeric AS b, '1e3'::numeric AS c,
       '  1.5  '::numeric AS d, '1.23e-4'::numeric AS e, '-0'::numeric AS f
FROM t;
