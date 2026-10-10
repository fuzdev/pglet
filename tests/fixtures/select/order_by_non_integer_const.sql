-- A bare non-integer constant as an ORDER BY key is a 42601 syntax error (Postgres
-- "non-integer constant in ORDER BY") — only an integer literal is a positional
-- column reference, and no other bare constant is a meaningful sort key. Distinct from
-- an out-of-range integer ordinal (42P10).
-- @setup
CREATE TABLE t (a INT, b TEXT);
INSERT INTO t VALUES (2, 'y'), (1, 'x');
-- @test
-- @expect-error: non-integer constant in ORDER BY
SELECT * FROM t ORDER BY 1.5;
