-- @setup
CREATE TABLE tag (id INT, name TEXT);
INSERT INTO tag VALUES
  (1, 'a'),
  (1, 'a'),
  (2, 'b'),
  (3, 'c'),
  (3, 'c');

-- @test
-- `DISTINCT ON (id)` with no ORDER BY: the ON key becomes the effective sort key
-- (ASC), so the output is grouped and ordered by `id`. The duplicated rows are
-- byte-identical, so the surviving row per group is deterministic here (Postgres
-- otherwise leaves the winner of a no-ORDER-BY group unspecified).
SELECT DISTINCT ON (id) id, name FROM tag;
