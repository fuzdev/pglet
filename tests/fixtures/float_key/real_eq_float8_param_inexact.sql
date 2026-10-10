-- A bound `float8` `1.1` against a `real` key: the cross-width `=` compares both as
-- `float8`, so the row holding the `real` `1.1` does not match.
-- @profile-lock
-- @param $1 float8 1.1
-- @setup
CREATE TABLE fk4 (k INT, x REAL);
CREATE INDEX fk4_x ON fk4 (x);
INSERT INTO fk4 VALUES (1, 1.5), (2, 1.1), (3, 'NaN'), (4, '-0'), (5, 16777216),
  (6, NULL), (7, '-NaN'), (8, 0);
-- @test
SELECT k FROM fk4 WHERE x = $1::float8 ORDER BY k;
