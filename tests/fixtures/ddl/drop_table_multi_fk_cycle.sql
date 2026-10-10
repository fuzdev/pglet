-- A mutual FK cycle drops in ONE statement and in neither single-target order — the
-- whole reason `RemoveRelations` resolves every relation before deleting any ("to
-- avoid unwanted DROP RESTRICT errors if one of the relations depends on another").
-- Each table's blocking dependent is declared on the other table, which is itself a
-- target, so the filter removes it and nothing is left to refuse.
-- `mfckeep` is the control, and it is why the count is `1` rather than `0`: a query
-- that saw nothing at all would answer `0`, so the surviving row is what makes the
-- two removals an answer rather than a broken lookup.
-- @setup
CREATE TABLE mfc1 (id INT PRIMARY KEY, o INT);
CREATE TABLE mfc2 (id INT PRIMARY KEY, o INT);
CREATE TABLE mfckeep (id INT);
ALTER TABLE mfc1 ADD CONSTRAINT mfc1_o_fkey FOREIGN KEY (o) REFERENCES mfc2(id);
ALTER TABLE mfc2 ADD CONSTRAINT mfc2_o_fkey FOREIGN KEY (o) REFERENCES mfc1(id);

-- @test
DROP TABLE mfc1, mfc2;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'mfc1'
       OR table_name = 'mfc2'
       OR table_name = 'mfckeep');
