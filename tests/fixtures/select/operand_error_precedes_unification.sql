-- **Source order between two different error families.** Postgres resolves an
-- expression in source order during parse analysis, so the leftmost bad subtree is the
-- one it reports — here the `LIKE` (`42883`), even though a later `COALESCE` is also
-- unresolvable. Its mirror image is `unification_error_precedes_operand.sql`.
-- Two separate whole-tree passes cannot satisfy both: whichever runs first wins
-- wherever its error sits. Only one bottom-up left-to-right walk deciding both per node
-- is right for both spellings.
-- @setup
CREATE TABLE opu (d DATE, k TEXT, x INT4);
INSERT INTO opu VALUES ('2026-01-01', 'a', 1);
-- @test
-- @expect-error: the leftmost error is the LIKE operand pair (42883)
SELECT d LIKE 'x%' AND COALESCE(k, x) IS NULL FROM opu;
