-- **A no-FROM select is a query level too, and it was the one level nothing re-walked.**
-- `SELECT (SELECT …)` never reaches `lower_select` — there are no `SelectParts` without a
-- FROM — so the correlated pass that closes a nested level's binding gate never ran over
-- its projection. The inner level's own scope was provisional (it is inside an expression
-- subquery), so nothing gated it at all, and over an **empty** inner relation the
-- eval-time rule had nothing to evaluate: this answered a row where 18.6 raises `42P01`.
--
-- The empty seed is the discriminator. Populate `nfs` and the eval-time gate catches it,
-- which is why the shape hid behind a green corpus.
-- @setup
CREATE TABLE nfs (k INT, v TEXT);

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
SELECT (SELECT 1 FROM nfs WHERE zzz.k = 1);
