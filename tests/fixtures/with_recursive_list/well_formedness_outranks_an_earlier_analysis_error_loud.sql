-- `checkWellFormedRecursion` judges every item before any is analysed, so a later
-- item's malformed recursion outranks an earlier item's missing column.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: the malformed recursion, not the column (42P19)
-- @expect-message
-- @test
WITH RECURSIVE q AS (SELECT nosuchcol FROM s),
  r(n) AS (SELECT n FROM r UNION ALL SELECT 1)
SELECT 1;
