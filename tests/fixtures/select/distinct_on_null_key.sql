-- @setup
CREATE TABLE nn (k TEXT, v INT);
INSERT INTO nn VALUES
  (NULL, 1),
  (NULL, 2),
  ('a', 3),
  ('a', 4);

-- @test
-- The ON key treats two NULLs as equal (like DISTINCT, unlike `=`), so the two
-- NULL-keyed rows collapse into a single group; `ORDER BY k, v` (NULLS LAST for
-- ASC) puts that group last with its smallest `v` surviving.
SELECT DISTINCT ON (k) k, v FROM nn ORDER BY k, v;
