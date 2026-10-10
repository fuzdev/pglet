-- A bare string constant as an ORDER BY key is likewise a 42601 "non-integer constant
-- in ORDER BY" (a text literal is not a column position).
-- @setup
CREATE TABLE t (a INT, b TEXT);
INSERT INTO t VALUES (2, 'y'), (1, 'x');
-- @test
-- @expect-error: non-integer constant in ORDER BY
SELECT * FROM t ORDER BY 'x';
