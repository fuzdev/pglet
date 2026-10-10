-- A bare non-integer constant as a DISTINCT ON key is a 42601 "non-integer constant in
-- DISTINCT ON" (only an integer literal is a positional column reference).
-- @setup
CREATE TABLE t (a INT, b TEXT);
INSERT INTO t VALUES (2, 'y'), (1, 'x');
-- @test
-- @expect-error: non-integer constant in DISTINCT ON
SELECT DISTINCT ON (1.5) * FROM t;
