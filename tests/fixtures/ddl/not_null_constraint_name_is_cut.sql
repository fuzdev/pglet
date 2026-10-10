-- A `NOT NULL` column's `pg_constraint` name is `<table>_<column>_not_null` through the
-- same cut: over a 60-byte table, a short column's (the table's name gives up the bytes),
-- a 40-byte column's (both do) and a primary key column's.
-- @setup
CREATE TABLE cac10_tttttttttttttttttttttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT NOT NULL, id INT PRIMARY KEY, k INT NOT NULL, free INT);
-- @test
SELECT conname::text AS conname, length(conname::text) AS len
FROM pg_constraint WHERE conname LIKE 'cac10\_%' AND contype = 'n' ORDER BY conname;
