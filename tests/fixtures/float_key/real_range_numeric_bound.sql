-- A range bound a `real` key cannot hold exactly is answered by the row filter, in
-- `float8`: the `real` `1.1` is above the `float8` `1.1`.
-- @profile-lock
-- @setup
CREATE TABLE fk4 (k INT, x REAL);
CREATE INDEX fk4_x ON fk4 (x);
INSERT INTO fk4 VALUES (1, 1.5), (2, 1.1), (3, 'NaN'), (4, '-0'), (5, 16777216),
  (6, NULL), (7, '-NaN'), (8, 0);
-- @test
SELECT 'gt' AS op, k FROM fk4 WHERE x > 1.1
UNION ALL
SELECT 'ge', k FROM fk4 WHERE x >= 1.1
UNION ALL
SELECT 'lt', k FROM fk4 WHERE x < 1.1
ORDER BY 1, 2;
