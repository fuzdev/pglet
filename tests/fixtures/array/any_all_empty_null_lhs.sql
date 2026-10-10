-- ANY/ALL over an EMPTY array short-circuits BEFORE the NULL-left check: an empty
-- array is vacuously ANY→false / ALL→true even when the left operand is NULL
-- (Postgres). A NULL left over a NON-empty array is still NULL.
-- @test
SELECT
  (NULL::int = ANY('{}'::int[]))    AS any_empty_null,      -- false
  (NULL::int = ALL('{}'::int[]))    AS all_empty_null,      -- true
  (5 = ANY('{}'::int[]))            AS any_empty_five,      -- false
  (NULL::int = ANY('{1,2}'::int[])) AS any_nonempty_null,   -- NULL
  (NULL::int = ALL('{1,2}'::int[])) AS all_nonempty_null    -- NULL
;
