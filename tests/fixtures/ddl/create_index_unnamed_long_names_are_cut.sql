-- A chosen name is at most 63 bytes (`makeObjectName`): the label and the underscores
-- are never cut, and the table's name and the key's share what is left, the **longer**
-- one giving up a byte at a time. Three keys over one 40-byte table name: a 40-byte column
-- (both cut, evenly), a 50-byte one (the column cut first, down to the table's length,
-- then both) and a 10-byte one (nothing cut). A cut that only shortens the tail, or only
-- the table's name, reads differently at each.
-- @setup
CREATE TABLE ciu15_tttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT, dooooooooooooooooooooooooooooooooooooooooooooooooo INT, eooooooooo INT);
CREATE INDEX ON ciu15_tttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo);
CREATE INDEX ON ciu15_tttttttttttttttttttttttttttttttttt (dooooooooooooooooooooooooooooooooooooooooooooooooo);
CREATE INDEX ON ciu15_tttttttttttttttttttttttttttttttttt (eooooooooo);
CREATE INDEX ON ciu15_tttttttttttttttttttttttttttttttttt (eooooooooo, cooooooooooooooooooooooooooooooooooooooo);
-- @test
SELECT indexname, length(indexname::text) AS len FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu15_tttttttttttttttttttttttttttttttttt' ORDER BY indexname;
