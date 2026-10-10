-- The accept side of `type_mismatch_multi_column_pairs_by_position_loud`.
-- @setup
CREATE TABLE ftp (x INT, y TEXT, PRIMARY KEY (x, y));
CREATE TABLE ftc (a TEXT, b INT, FOREIGN KEY (a, b) REFERENCES ftp (y, x));

-- @test
SELECT conname, contype FROM pg_constraint WHERE contype = 'f' ORDER BY conname;
