-- A scalar subquery with no `FROM` projecting `ctz_q.b`, where `ctz_q` is the enclosing
-- level's alias for `ctz_p` and also the name of a catalog table with a `text` column
-- `b`. The sub-plan introduces no relation, so the qualifier binds to the alias: the
-- fold's type is `ctz_p.b`'s integer and the literal branch `'x'` is 18.6's `22P02`.
-- Read by the catalog table's name the fold was `text` and answered.
-- @setup
CREATE TABLE ctz_p (a INT, b INT);
CREATE TABLE ctz_q (c INT, b TEXT);
INSERT INTO ctz_p VALUES (1, 10), (2, 20);
-- @expect-error: a literal branch read as the alias's integer column — 22P02
-- @test
SELECT a, coalesce((SELECT ctz_q.b), 'x') FROM ctz_p ctz_q ORDER BY a;
