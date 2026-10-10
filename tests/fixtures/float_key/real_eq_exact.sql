-- A `real` key: a probe value the narrow width holds exactly is served by the index.
-- @profile-lock
-- @setup
CREATE TABLE fk4 (k INT, x REAL);
CREATE INDEX fk4_x ON fk4 (x);
INSERT INTO fk4 VALUES (1, 1.5), (2, 1.1), (3, 'NaN'), (4, '-0'), (5, 16777216),
  (6, NULL), (7, '-NaN'), (8, 0);
-- @test
SELECT 'half' AS probe, k FROM fk4 WHERE x = 1.5
UNION ALL
SELECT 'zero', k FROM fk4 WHERE x = 0
UNION ALL
SELECT 'big', k FROM fk4 WHERE x = 16777216
ORDER BY 1, 2;
