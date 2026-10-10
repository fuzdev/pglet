-- @setup
CREATE TABLE sdupe1 (c INT, x INT);
CREATE TABLE sdupe2 (k INT);
INSERT INTO sdupe1 VALUES (1, 10), (2, 20);
INSERT INTO sdupe2 VALUES (1), (2);

-- The **refuse** polarity: two target-list entries under one name that are *different*
-- expressions. `findTargetlistEntrySQL92` compares the two with `equal()` and raises
-- `42702` when they differ, which is the half
-- `nested_sort_key_duplicate_output_entries_resolve.sql` must not be allowed to widen
-- away — dropping the comparison entirely and resolving on the first match is what let a
-- bare key two of a join's columns claim answer silently, and it *wrote*
-- (`subquery_sort_key_ambiguous_output_name_loud.sql`, and the row-survival halves in
-- `crates/pglet/tests/scope_relation_identity.rs`).
--
-- The subquery is `WHERE false`, so the key is never evaluated: the refusal is the
-- plan-time gate's, not the eval-time resolver's.
-- @expect-error: two different expressions under one output name are 42702
-- @test
SELECT k FROM sdupe2 WHERE EXISTS (SELECT c AS q, x AS q FROM sdupe1 WHERE false ORDER BY q);
