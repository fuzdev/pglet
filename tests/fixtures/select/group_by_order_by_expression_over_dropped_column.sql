-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- A key does not have to *be* the dropped column to need the junk entry: any
-- expression over one does. `k*2` is not a bare name, so `findTargetlistEntrySQL92`
-- never reaches it and it resolves against the FROM relations like any other
-- expression.
-- @test
SELECT count(*) FROM gj GROUP BY k ORDER BY k * 2 DESC;
