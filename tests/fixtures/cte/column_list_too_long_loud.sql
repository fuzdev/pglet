-- A list longer than the body's output is `42P10` at `WITH` time, with `analyzeCTE`'s
-- own wording (`WITH query "…"`, not `parse_relation.c`'s `table "…"`).
-- @expect-error: more aliases than the CTE body has columns
-- @expect-message
-- @test
WITH c(w, x, y, z) AS (SELECT 1, 2, 3) SELECT * FROM c;
