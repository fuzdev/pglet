-- @setup
CREATE TABLE sdup1 (c INT, x INT);
CREATE TABLE sdup2 (k INT);
INSERT INTO sdup1 VALUES (1, 10), (2, 20);
INSERT INTO sdup2 VALUES (1), (2);

-- The **accept** polarity of `nested_sort_key_duplicate_output_entries_loud.sql`, and
-- the half a claimant *count* gets wrong. `findTargetlistEntrySQL92` scans the whole
-- target list and raises `42702` only when two matching entries are not `equal()` — its
-- own comment: "we allow SELECT a, a FROM table ORDER BY a but not SELECT a AS b, b
-- FROM table ORDER BY b". Counting entries by name refuses both, and every column here
-- was `42702` against 18.6's `f`.
--
-- Five spellings of *one* column written twice, because the identity is normalized in
-- three separate ways and each was measured failing on its own:
--
-- - `*, c` — wildcard expansion emits a **qualified** reference, so the star's entry and
--   the written one differ in spelling and not in what they denote;
-- - `c AS c, c` — an `AS` label lives on Postgres's `TargetEntry`, not on the expression
--   `equal()` compares;
-- - `sdup1.c, c` — the qualified/bare pair directly;
-- - `GROUP BY` — the target list of a `Plan::Aggregate` rather than a `Plan::Project`;
-- - `DISTINCT ON` — read **above** the sort, so its input is the sort and the projecting
--   node is one wrapper further down.
--
-- Every subquery is `WHERE false`, so the sorted input is empty and `EXISTS` is `f` on
-- both engines; that also keeps the key away from the eval-time resolver, whose own
-- stricter answer over a row carrying two same-named columns is a separate, row-bearing
-- divergence recorded in `EDGES.md`.
--
-- The key had to be **nested** for any of this to assert anything while a level's own
-- lowering resolved its sort keys before the projection existed, leaving only the
-- correlated re-walk with a claimant set. Both phases carry one now, over the same
-- `entry_identity`; the top-level spellings are
-- `select/order_by_star_and_bare_duplicate_resolve.sql` and its `AS`-label twin, and this
-- fixture stays because the two derivations can regress independently.
-- @test
SELECT k,
       EXISTS (SELECT *, c FROM sdup1 WHERE false ORDER BY c) AS star_then_bare,
       EXISTS (SELECT c AS c, c FROM sdup1 WHERE false ORDER BY c) AS alias_then_bare,
       EXISTS (SELECT sdup1.c, c FROM sdup1 WHERE false ORDER BY c) AS qualified_then_bare,
       EXISTS (SELECT c, c FROM sdup1 WHERE false GROUP BY c ORDER BY c) AS grouped_twice,
       EXISTS (SELECT DISTINCT ON (c) sdup1.c, c FROM sdup1 WHERE false) AS distinct_on_key
FROM sdup2
ORDER BY k;
