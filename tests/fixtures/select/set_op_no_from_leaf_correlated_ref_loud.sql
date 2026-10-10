-- A no-FROM leaf of a set op ran **no** correlated pass. `parser::select`'s
-- `finalize_leaf` lowered a `SelectLeaf::NoFrom` operand as `Ok(*plan)` where its
-- `parse_select_inner` twin calls `resolve_correlated_subplans` on the identical leaf, so
-- a level inside such a leaf kept the open scope its own lowering gave it and nothing ever
-- closed it. This answered two rows (`1` and NULL) against 18.6's `42P01`.
--
-- The class needs no sort or `DISTINCT ON` key at all — a plain correlated reference rides
-- it, which is what puts it on the leaf rather than on any key gate. Two controls localise
-- it: drop the `UNION ALL` and the same leaf lowers through `parse_select_inner`, which
-- makes the call; wrap the set op in `IN (…)` and the enclosing statement's re-walk
-- reaches it. Both refused with these SQLSTATEs before the repair.
-- @setup
CREATE TABLE sop_et (k INT);

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
SELECT 1 AS z UNION ALL SELECT (SELECT 1 FROM sop_et WHERE zzz.k = 1);
