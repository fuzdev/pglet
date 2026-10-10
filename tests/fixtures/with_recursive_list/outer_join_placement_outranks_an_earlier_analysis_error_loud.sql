-- The outer-join placement rule is part of that phase too.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: the placement, not the column (42P19)
-- @expect-message
-- @test
WITH RECURSIVE q AS (SELECT nosuchcol FROM s),
  r(n) AS (SELECT 1 UNION ALL SELECT s.x FROM s LEFT JOIN r ON true)
SELECT 1;
