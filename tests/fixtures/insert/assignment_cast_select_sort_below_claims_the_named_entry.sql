-- The refusing half of the pair `assignment_cast_select_sort_below_claims_by_position.sql`
-- opens: the same source, the same key, the target columns swapped. `ORDER BY p`
-- resolves the **first** entry to `text`, which now meets the `BOOL` column — `42804` on
-- 18.6, where the accept twin is ordinary SQL.
--
-- Read together the pair pins the position and nothing else: no single-entry fixture can
-- tell "claim the entry the key names" from "claim every entry equal to it", and the two
-- entries here are equal.
-- @setup
CREATE TABLE acdq (b BOOL, t TEXT);
CREATE TABLE acdqsrc (n INT);
INSERT INTO acdqsrc VALUES (1), (2);

-- @expect-error: column "b" is of type boolean but expression is of type text (42804)
-- @test
INSERT INTO acdq (b, t) SELECT 't' AS p, 't' AS q FROM acdqsrc ORDER BY p, n;
