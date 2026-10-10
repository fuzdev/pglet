-- `COALESCE` over a scalar subquery projecting an enclosing `INT` column, the inner
-- table carrying a same-named `text` column: the fold's type is the outer column's, so
-- the literal branch is read as an integer and `'x'` is 18.6's `22P02`.
-- @setup
CREATE TABLE ctf_p (a INT, b INT);
CREATE TABLE ctf_q (c INT, b TEXT);
INSERT INTO ctf_p VALUES (1, 10), (2, 20);
INSERT INTO ctf_q VALUES (5, 'zz');
-- @expect-error: a literal branch read as the enclosing integer column — 22P02
-- @test
SELECT a, coalesce((SELECT ctf_p.b FROM ctf_q), 'x') FROM ctf_p ORDER BY a;
