-- A constraint declared without a name is named as 18.6 names it (`makeObjectName`): cut to
-- 63 bytes, the label and the underscores never cut, the table's name and the second name
-- sharing what is left with the **longer** giving up bytes first. Every kind a `CREATE
-- TABLE` mints, column-level and table-level, over a 60-byte table name: a primary key's
-- and a multi-column `CHECK`'s second name is absent (`<table>_pkey`, `<table>_check`), a
-- `UNIQUE`'s and a `FOREIGN KEY`'s is its columns joined in **key** order, which is not
-- the table's column order here.
-- @setup
CREATE TABLE cac1p (y INT, x INT, PRIMARY KEY (x, y), UNIQUE (y));
CREATE TABLE cac1_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt (
  dooooooooooooooooooooooooooooooooooooooooooooooooo INT CHECK (dooooooooooooooooooooooooooooooooooooooooooooooooo > 0) UNIQUE,
  cooooooooooooooooooooooooooooooooooooooo INT REFERENCES cac1p (y),
  id INT PRIMARY KEY,
  eooooooooo INT,
  UNIQUE (eooooooooo, cooooooooooooooooooooooooooooooooooooooo),
  CHECK (cooooooooooooooooooooooooooooooooooooooo > eooooooooo),
  CHECK (eooooooooo > 1),
  FOREIGN KEY (cooooooooooooooooooooooooooooooooooooooo, eooooooooo) REFERENCES cac1p,
  UNIQUE (cooooooooooooooooooooooooooooooooooooooo, eooooooooo));
-- @test
SELECT contype::text AS contype, conname::text AS conname, length(conname::text) AS len
FROM pg_constraint WHERE conname LIKE 'cac1\_%' AND contype <> 'n' ORDER BY contype, conname;
