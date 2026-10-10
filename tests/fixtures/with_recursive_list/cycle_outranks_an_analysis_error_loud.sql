-- The dependency sort runs before any body is analysed, so a cycle outranks an
-- earlier item's own error.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: the cycle, not the column (0A000)
-- @expect-message
-- @test
WITH RECURSIVE z AS (SELECT nosuchcol FROM s),
  a AS (SELECT * FROM b),
  b AS (SELECT * FROM a)
SELECT 1;
