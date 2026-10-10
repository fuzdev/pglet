-- The expression half of the rule diverged identically, so fixing only the column half is
-- half a fix. 18.6 keeps the plain attributes in a `Bitmapset` and the key expressions in
-- a list, and matches the list by membership in both directions (`list_member` one way,
-- `list_difference` the other) — set equality there too. pglet compared the whole key list
-- element-wise and answered `42P10`.
--
-- pglet's catalog holds a mixed key as one expression list, so the split into the two
-- halves is made where the comparison is, not where the index is stored.
-- @setup
CREATE TABLE oare (x TEXT, y INT);
CREATE UNIQUE INDEX oare_lx ON oare ((lower(x)), y);
INSERT INTO oare VALUES ('A', 1);
INSERT INTO oare VALUES ('a', 1) ON CONFLICT (y, lower(x)) DO NOTHING;

-- @test
SELECT x, y FROM oare ORDER BY x;
