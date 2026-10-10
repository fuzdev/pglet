-- The same inside a `LIMIT` followed by an `OFFSET`: the `OFFSET` refusal, which 18.6
-- tests ahead of the `LIMIT` one, is the verdict.
-- @setup
CREATE TABLE t (a INT);
-- @expect-error: OFFSET in a recursive query (0A000)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 LIMIT (SELECT nosuch FROM t) OFFSET 1)
SELECT * FROM r;
