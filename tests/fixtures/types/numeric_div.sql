-- NUMERIC division chooses its result scale like Postgres `select_div_scale`
-- (at least 16 significant digits, never below either input's scale).
-- @setup
CREATE TABLE t (id INT, a NUMERIC, b NUMERIC);
INSERT INTO t VALUES (1, 10, 3), (2, 1, 4), (3, 7, 10), (4, 22, 7);

-- @test
SELECT id, a / b AS quotient FROM t ORDER BY id;
