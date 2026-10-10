-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- The accept-polarity control for the junk rule: an aggregate the select list *does*
-- carry is not junk at all — Postgres reuses that target entry
-- (`findTargetlistEntrySQL99`), and pglet resolves the key to the output column it
-- denotes. Two columns come back, not three, which is what says no junk column
-- leaked to the wire.
-- @test
SELECT k, count(*) FROM gj GROUP BY k ORDER BY count(*) DESC;
