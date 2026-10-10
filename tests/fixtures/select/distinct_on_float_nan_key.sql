-- @setup
CREATE TABLE fnan (k FLOAT8, v INT);
INSERT INTO fnan VALUES
  ('NaN', 1),
  ('NaN', 2);

-- @test
-- The DISTINCT ON key treats float `NaN` as not-distinct from `NaN` (like DISTINCT,
-- unlike the `=` operator where `NaN = NaN` is false), so the two NaN-keyed rows
-- collapse to one group; the `v` tiebreak keeps the smaller.
SELECT DISTINCT ON (k) k, v FROM fnan ORDER BY k, v;
