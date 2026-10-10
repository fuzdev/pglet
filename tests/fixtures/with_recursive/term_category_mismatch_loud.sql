-- The category-mismatch leg of the recursive-CTE type check, and an honest note about
-- what it is worth: **it is a conformance assertion, not a regression lock.** Reverting
-- the change that routes this check through `select_common_type` leaves it green,
-- because the raw fold also produced `42804` here — only the *message* differed
-- ("has type integer in non-recursive term but type unknown overall" rather than
-- Postgres's "UNION types integer and text cannot be matched"), and `@expect-error`
-- compares the SQLSTATE, not the text. `with_recursive/term_no_common_type_loud.sql` is
-- the same-category twin that does redden (`42846` vs the raw fold's `42804`), and
-- `with_recursive/type_mismatch_loud.sql` covers the pair that unifies and so belongs to
-- the recursive check proper.
-- @expect-error: the two UNION terms are in different type categories (42804)
-- @test
WITH RECURSIVE r(x) AS (
  SELECT 1
  UNION ALL
  SELECT 'a'::text FROM r WHERE false
)
SELECT * FROM r;
