-- The recursive path's own `42P10`, which shares `analyzeCTE`'s wording with the
-- ordinary one — it used to render `parse_relation.c`'s `table "…"` instead.
-- @expect-error: more aliases than the seed term has columns
-- @expect-message
-- @test
WITH RECURSIVE r(n, m) AS (
  SELECT 1
  UNION ALL
  SELECT n + 1 FROM r WHERE n < 3
)
SELECT * FROM r;
