-- A bare integer `ORDER BY <n>` is a positional reference to the Nth output
-- column (SQL semantics), resolved by index. Here `ORDER BY 1` orders by the
-- first output column `b` (x, y, z), not by `a`.
-- @setup
CREATE TABLE t (a INT, b TEXT);
INSERT INTO t VALUES (3, 'x'), (1, 'z'), (2, 'y');

-- @test
SELECT b, a FROM t ORDER BY 1;
