-- A fractional LIMIT/OFFSET count coerces via numeric→int8, rounding half AWAY from
-- zero (Postgres), not ties-to-even: `LIMIT 2.5` keeps 3 rows (ties-to-even would keep
-- 2), and `OFFSET 0.5` skips 1 (ties-to-even would skip 0).
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1), (2), (3), (4), (5);
-- @test
SELECT id FROM t ORDER BY id OFFSET 0.5 LIMIT 2.5;
