-- The **accept** side of the recursive-term type check, and it is a conformance repair
-- rather than a new rule: `abs(n)` over the working table used to be typed against the
-- catalog, which cannot name a CTE, so `abs`'s argument was unknown, `abs(<unknown>)`
-- resolved to `numeric` (its documented no-argument fallback), `numeric + 1` folded
-- `numeric`, and the check reported `42804 … has type integer in non-recursive term but
-- type numeric overall` on ordinary SQL 18.6 runs. Typing the term against the working
-- table's own columns — the same namespace `output_columns` uses for the very same
-- expressions — gives `abs(int4) + 1` = `int4`, which is what the seed declares.
--
-- Its reject-side twin is `with_recursive/join_sourced_term_widens_loud.sql`; each needs
-- its own file (one `@test` per fixture, and the twin is an `@expect-error`), and the
-- pair reddens under opposite halves of the same revert.
-- @test
WITH RECURSIVE wtc(n) AS (
  SELECT 1
  UNION ALL
  SELECT abs(n) + 1 FROM wtc WHERE n < 3
)
SELECT n FROM wtc ORDER BY n;
