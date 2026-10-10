-- `text` and `varchar` are one string family for comparison (pglet stores both as
-- `Value::Text`, so the pair is reachable only through a declared column type).
-- @setup
CREATE TABLE t (a TEXT, b VARCHAR(10), c CHAR(3));
INSERT INTO t VALUES ('abc', 'abc', 'abc');
-- @test
SELECT a = b AS t_v, b = c AS v_c, a = c AS t_c, a < 'abd' AS lt FROM t;
