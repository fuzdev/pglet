-- The system-column list is consulted on the **folded** name, so an unquoted `CTID` is
-- refused (`create_table_system_column_name_folds_the_unquoted_spelling.sql`) while a
-- quoted `"CTID"` never reaches the list at all — a distinct name, and an ordinary
-- column on both engines.
-- The `ADD COLUMN` spelling is in the same setup, since both routes read one predicate.
-- @setup
CREATE TABLE ctqs ("CTID" INT);
ALTER TABLE ctqs ADD COLUMN "XMIN" TEXT;
INSERT INTO ctqs VALUES (3, 'z');
-- @test
SELECT "CTID", "XMIN" FROM ctqs;
