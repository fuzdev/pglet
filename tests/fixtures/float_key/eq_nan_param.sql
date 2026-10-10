-- Every NaN is one key, and it equals itself in the opclass: a bound `NaN` finds the
-- row written as `'NaN'` and the one `Infinity - Infinity` produced, whose sign bit is
-- set on 18.6's own platform.
-- @profile-lock
-- @param $1 float8 NaN
-- @setup
CREATE TABLE fk8 (id INT, tag TEXT, x FLOAT8);
CREATE INDEX fk8_x ON fk8 (x);
INSERT INTO fk8 VALUES
  (1, 'a', 1.5), (2, 'b', '-Infinity'), (3, 'c', 'NaN'), (4, 'd', 'Infinity'),
  (5, 'e', '-0'), (6, 'f', 0), (7, 'g', -2.5), (8, 'h', NULL), (9, 'i', 1e300),
  (10, 'j', -1e-300), (11, 'k', 'Infinity'::float8 - 'Infinity'::float8),
  (12, 'l', 5e-324), (13, 'm', -5e-324);
-- @test
SELECT id, x::text AS shown FROM fk8 WHERE x = $1 ORDER BY id;
