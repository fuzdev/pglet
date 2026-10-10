-- NUMERIC NaN propagates through arithmetic (NaN op x = NaN).
-- @setup
CREATE TABLE t (a NUMERIC);
INSERT INTO t VALUES ('NaN');
-- @test
SELECT a + 1 AS s, a * 0 AS p, a - a AS d FROM t;
