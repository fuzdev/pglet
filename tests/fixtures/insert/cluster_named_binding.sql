-- @setup
CREATE TABLE t (a INT, b TEXT);
INSERT INTO t (b, a) VALUES ('x', 1);

-- @test
SELECT a, b FROM t;
