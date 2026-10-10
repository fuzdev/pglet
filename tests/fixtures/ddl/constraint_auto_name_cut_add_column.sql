-- `ALTER TABLE … ADD COLUMN`'s inline constraints are cut as a `CREATE TABLE`'s are: an
-- inline `UNIQUE`, `CHECK` and `REFERENCES` on a 40-byte column of a 60-byte table, and a
-- second column whose name differs from the first only past the cut — so each of its
-- three names meets the first column's and takes the digit, on the label and inside the
-- 63 bytes.
-- @setup
CREATE TABLE cac2_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt (id INT PRIMARY KEY);
ALTER TABLE cac2_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD COLUMN cooooooooooooooooooooooooooooooooooooooox INT UNIQUE CHECK (cooooooooooooooooooooooooooooooooooooooox > 0) REFERENCES cac2_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt;
ALTER TABLE cac2_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD COLUMN coooooooooooooooooooooooooooooooooooooooy INT CHECK (coooooooooooooooooooooooooooooooooooooooy > 0) REFERENCES cac2_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt UNIQUE;
-- @test
SELECT contype::text AS contype, conname::text AS conname, length(conname::text) AS len
FROM pg_constraint WHERE conname LIKE 'cac2\_%' AND contype <> 'n' ORDER BY contype, conname;
