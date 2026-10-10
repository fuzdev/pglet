-- The accept-side control on the system-column list, and the one plausible **extra**
-- member: `oid` has not been a per-row system column since it stopped being one, so it
-- is an ordinary column name on both engines, at `CREATE TABLE` and at
-- `ALTER TABLE … ADD COLUMN` alike. The `ADD COLUMN` half is `ctoid2`'s, since adding
-- `oid` to `ctoid` would collide with the column already there and test the wrong gate.
-- @setup
CREATE TABLE ctoid (oid INT);
ALTER TABLE ctoid ADD COLUMN oid2 INT;
INSERT INTO ctoid VALUES (7, 8);
CREATE TABLE ctoid2 (x INT);
ALTER TABLE ctoid2 ADD COLUMN oid INT;
-- @test
SELECT oid, oid2 FROM ctoid;
