-- @setup
CREATE TABLE fdist (k FLOAT8);
INSERT INTO fdist VALUES
  ('NaN'),
  ('NaN'),
  (1.5),
  (1.5),
  (2.5);

-- @test
-- Plain `SELECT DISTINCT` shares the DISTINCT-family key equality, so its float
-- `NaN` rows collapse too (`NaN` not-distinct from `NaN`): five rows dedup to three
-- distinct values. Counted through a derived table so the assertion is order-
-- independent (the NaN vs finite sort position is not exercised here).
SELECT count(*) AS n FROM (SELECT DISTINCT k FROM fdist) s;
