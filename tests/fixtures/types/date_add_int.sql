-- `date ± int → date` (a day shift) and the commutative `int + date → date`,
-- byte-for-byte vs Postgres. Exercises both the int2 (column `n`) and int4
-- (literal) operand widths; the day shift is Julian-day based and range-checked.
-- @setup
CREATE TABLE t (d DATE, n SMALLINT);
INSERT INTO t VALUES ('2024-06-15', 10), ('2024-01-01', -1), ('2024-12-31', 1);

-- @test
SELECT d,
       d + n   AS plus_int2,
       d + 100 AS plus_int4,
       100 + d AS int4_plus,
       d - n   AS minus_int2,
       d - 400 AS minus_int4
FROM t
ORDER BY d ASC;
