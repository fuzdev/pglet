-- And it outranks a later item's own analysis error.
-- @setup
CREATE TABLE s (x INT);
-- @expect-error: the first item's arity error (42P10)
-- @expect-message
-- @test
WITH RECURSIVE c(a, b) AS (SELECT 1),
  q AS (SELECT nosuchcol FROM s)
SELECT 1;
