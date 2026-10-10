-- The classic case: a tree traversal whose recursive term JOINs the base table to
-- the recursive CTE name, accumulating a depth. Column names/types come from the
-- `chain(id, name, depth)` alias list and the non-recursive term.
-- @setup
CREATE TABLE org (id INT, parent INT, name TEXT);
INSERT INTO org VALUES (1, NULL, 'ceo'), (2, 1, 'vp1'), (3, 1, 'vp2'), (4, 2, 'eng'), (5, 4, 'intern');
-- @test
WITH RECURSIVE chain(id, name, depth) AS (
  SELECT id, name, 1 FROM org WHERE parent IS NULL
  UNION ALL
  SELECT o.id, o.name, c.depth + 1 FROM org o JOIN chain c ON o.parent = c.id
)
SELECT id, name, depth FROM chain ORDER BY depth, id;
