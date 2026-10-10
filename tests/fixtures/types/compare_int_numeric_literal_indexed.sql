-- A decimal literal against an **indexed** integer column. Postgres compares in the
-- wider type (`int4 > 1.5` resolves as numeric), so `n = 2` is `> 1.5` and no integer
-- `= 1.5`. An index over `n` keys on int4, and rounding `1.5` to the key type is not
-- order-preserving — `> 1.5` became `> 2` (losing the row `2`) and `= 1.5` became
-- `= 2` (inventing one). The probe now declines an inexact coercion and scans, so the
-- accelerated build agrees with the stripped one and with Postgres.
-- @profile-lock
-- @setup
CREATE TABLE t (n INT, tag TEXT);
INSERT INTO t VALUES (1, 'a'), (2, 'b');
CREATE INDEX t_n ON t (n);
-- @test
SELECT 'gt' AS op, tag FROM t WHERE n > 1.5
UNION ALL
SELECT 'eq', tag FROM t WHERE n = 1.5
UNION ALL
SELECT 'le', tag FROM t WHERE n <= 1.5
ORDER BY 1, 2;
