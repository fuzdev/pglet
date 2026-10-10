-- Range probes over a `float8` key read the index in `float8_ops` order: negatives
-- below zero, the subnormals either side of it, `Infinity` below `NaN`.
-- @profile-lock
-- @setup
CREATE TABLE fk8 (id INT, tag TEXT, x FLOAT8);
CREATE INDEX fk8_x ON fk8 (x);
INSERT INTO fk8 VALUES
  (1, 'a', 1.5), (2, 'b', '-Infinity'), (3, 'c', 'NaN'), (4, 'd', 'Infinity'),
  (5, 'e', '-0'), (6, 'f', 0), (7, 'g', -2.5), (8, 'h', NULL), (9, 'i', 1e300),
  (10, 'j', -1e-300), (11, 'k', 'Infinity'::float8 - 'Infinity'::float8),
  (12, 'l', 5e-324), (13, 'm', -5e-324);
-- @test
SELECT 'gt' AS op, id FROM fk8 WHERE x > 1.5
UNION ALL
SELECT 'lt0', id FROM fk8 WHERE x < 0
UNION ALL
SELECT 'le0', id FROM fk8 WHERE x <= 0
UNION ALL
SELECT 'between', id FROM fk8 WHERE x > -1 AND x < 2
ORDER BY 1, 2;
