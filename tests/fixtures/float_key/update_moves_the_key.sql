-- An `UPDATE` of an indexed float moves its entry: the old key no longer finds the row
-- and the new one does.
-- @profile-lock
-- @setup
CREATE TABLE fk8 (id INT, tag TEXT, x FLOAT8);
CREATE INDEX fk8_x ON fk8 (x);
INSERT INTO fk8 VALUES
  (1, 'a', 1.5), (2, 'b', '-Infinity'), (3, 'c', 'NaN'), (4, 'd', 'Infinity'),
  (5, 'e', '-0'), (6, 'f', 0), (7, 'g', -2.5), (8, 'h', NULL), (9, 'i', 1e300),
  (10, 'j', -1e-300), (11, 'k', 'Infinity'::float8 - 'Infinity'::float8),
  (12, 'l', 5e-324), (13, 'm', -5e-324);
UPDATE fk8 SET x = 2.5 WHERE id = 1;
UPDATE fk8 SET x = 'NaN' WHERE id = 7;
DELETE FROM fk8 WHERE id = 3;
-- @test
SELECT 'old' AS probe, id FROM fk8 WHERE x = 1.5
UNION ALL
SELECT 'new', id FROM fk8 WHERE x = 2.5
UNION ALL
SELECT 'above', id FROM fk8 WHERE x > 1e308
ORDER BY 1, 2;
