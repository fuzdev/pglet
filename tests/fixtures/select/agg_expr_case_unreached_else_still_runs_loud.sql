-- 18.6 computes every aggregate that survives constant folding for every group, whether
-- or not the expression above it reads the value: the `ELSE` is never taken, and its
-- `sum` still divides by the `b = 0` row.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN count(*) > 0 THEN max(a) ELSE sum(1/b) END FROM u;
