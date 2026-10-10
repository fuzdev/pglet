-- A recursive CTE whose two terms have **no** common type at all is the set operation's
-- own error, not the recursive-column one: Postgres's `transformSetOperationTree` folds
-- the terms before `analyzeCTE` ever compares them, so `time` beside `timestamp` is
-- `42846 UNION could not convert …`. pglet folded the pair raw, read the fold's `None` as
-- "unknown", and reported `42804 recursive query "r" column 1 has type time in
-- non-recursive term but type unknown overall` — a message describing a resolution that
-- never happened.
-- @expect-error: the two UNION terms have no common type (42846)
-- @test
WITH RECURSIVE r(x) AS (
  SELECT '10:00:00'::TIME
  UNION ALL
  SELECT '2026-01-01 10:00:00'::TIMESTAMP FROM r WHERE false
)
SELECT * FROM r;
