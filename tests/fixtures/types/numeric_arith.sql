-- NUMERIC arithmetic: add/sub display at max(scale), multiply at the sum of
-- scales (exact), and int operands promote to numeric.
-- @setup
CREATE TABLE t (id INT, a NUMERIC, b NUMERIC);
INSERT INTO t VALUES (1, 1.50, 2.25), (2, 10, 3), (3, 0.1, 0.2);

-- @test
SELECT id, a + b AS sum, a - b AS diff, a * b AS prod, a + 5 AS plus_int
FROM t ORDER BY id;
