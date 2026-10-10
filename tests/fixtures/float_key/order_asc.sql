-- An ordered scan over a NOT NULL `float8` key reads the index instead of sorting, and
-- the index order is `float8_ops`': `-Infinity`, the negatives, zero, the positives,
-- `Infinity`, `NaN`.
-- @profile-lock
-- @setup
CREATE TABLE fko (g INT, x FLOAT8 NOT NULL);
CREATE INDEX fko_x ON fko (x);
INSERT INTO fko VALUES (1, 2.5), (2, 'NaN'), (3, '-Infinity'), (4, 0), (6, 'Infinity'),
  (7, -1.5), (8, 1e-320), (9, -1e-320);
-- @test
SELECT g, x::text AS shown FROM fko ORDER BY x;
