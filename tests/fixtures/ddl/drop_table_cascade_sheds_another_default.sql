-- `CASCADE` sheds the dependent **default**. The other table survives with its column,
-- its rows and its type; only `column_default` goes. The post-CASCADE insert is part
-- of the assertion — the column is still writable, defaulting to NULL now.
-- @setup
CREATE TABLE dtcad_owner (a SERIAL, b INT);
CREATE TABLE dtcad_user (x INT DEFAULT nextval('dtcad_owner_a_seq'));
INSERT INTO dtcad_user (x) VALUES (DEFAULT), (DEFAULT);
DROP TABLE dtcad_owner CASCADE;
INSERT INTO dtcad_user (x) VALUES (DEFAULT);
-- @test
SELECT x FROM dtcad_user ORDER BY x NULLS LAST;
