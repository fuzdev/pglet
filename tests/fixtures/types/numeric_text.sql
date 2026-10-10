-- The text-format wire encoder renders NUMERIC with its display scale (checked
-- over the simple-query protocol, which the default binary path skips).
-- @setup
CREATE TABLE t (id INT, a NUMERIC(10,2), b NUMERIC);
INSERT INTO t VALUES (1, 1.5, 0.001), (2, -12.30, 1000000);

-- @text
-- @test
SELECT id, a, b FROM t ORDER BY id;
