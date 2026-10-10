-- @setup
CREATE TABLE item (grp TEXT, sub INT, label TEXT);
INSERT INTO item VALUES
  ('a', 2, 'x'),
  ('a', 2, 'x'),
  ('a', 1, 'y'),
  ('b', 1, 'z');

-- @test
-- `DISTINCT ON (grp, sub)` with only `ORDER BY grp`: `sub` is not in the ORDER BY,
-- so it is appended to the effective sort key ASC (Postgres's transformDistinctOnClause
-- rule) rather than erroring. The duplicate `(a, 2, 'x')` row collapses to one.
SELECT DISTINCT ON (grp, sub) grp, sub, label FROM item ORDER BY grp;
