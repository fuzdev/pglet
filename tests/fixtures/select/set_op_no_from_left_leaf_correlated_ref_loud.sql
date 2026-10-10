-- The left operand reaches `finalize_leaf` through a different call site
-- (`parse_select_inner`'s own, rather than `parse_setop_operand`'s), so the repair has to
-- be in the shared function and not at one caller.
-- @setup
CREATE TABLE sop_et (k INT);

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
SELECT (SELECT 1 FROM sop_et WHERE zzz.k = 1) UNION ALL SELECT 1 AS z;
