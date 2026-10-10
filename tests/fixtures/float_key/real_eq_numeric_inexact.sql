-- `real = 1.1` compares as `float8` on 18.6, and the `real` nearest `1.1` is not the
-- `float8` nearest it: no row. The probe declines a value the key type cannot hold
-- exactly and the row filter answers, so rounding the probe to `real` cannot invent
-- the match.
-- @profile-lock
-- @setup
CREATE TABLE fk4 (k INT, x REAL);
CREATE INDEX fk4_x ON fk4 (x);
INSERT INTO fk4 VALUES (1, 1.5), (2, 1.1), (3, 'NaN'), (4, '-0'), (5, 16777216),
  (6, NULL), (7, '-NaN'), (8, 0);
-- @test
SELECT 'numeric' AS probe, k FROM fk4 WHERE x = 1.1
UNION ALL
SELECT 'int', k FROM fk4 WHERE x = 16777217
UNION ALL
SELECT 'real', k FROM fk4 WHERE x = 1.1::real
ORDER BY 1, 2;
