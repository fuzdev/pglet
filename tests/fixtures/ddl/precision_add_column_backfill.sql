-- An `ADD COLUMN … DEFAULT` backfill runs through the same store finalizer, so the
-- existing rows get the **rounded** value rather than the literal as written.
-- @setup
CREATE TABLE tpb (id int);
INSERT INTO tpb VALUES (1), (2);
ALTER TABLE tpb ADD COLUMN a timestamp(3) DEFAULT '2020-01-02 03:04:05.6789';
ALTER TABLE tpb ADD COLUMN c interval(3) DEFAULT '1.23456 secs';
-- @test
SELECT id, a, c FROM tpb ORDER BY id;
