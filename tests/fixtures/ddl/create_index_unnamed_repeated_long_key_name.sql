-- A repeated 63-byte key column: 18.6 gives the repeat a digit cut into the part's own 63
-- bytes (`ChooseIndexColumnNames`), and the joined pair is then cut like any other long key
-- — to less than the first occurrence alone, so the digit is past the cut and both
-- statements choose the same base. The second is therefore the first's name with the index
-- name's own digit. A control: no name here shows the part's digit, on either engine.
-- @setup
CREATE TABLE ciu18 (xyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy INT);
CREATE INDEX ON ciu18 (xyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy, xyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy);
CREATE INDEX ON ciu18 (xyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy);
-- @test
SELECT indexname, length(indexname::text) AS len FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu18' ORDER BY indexname;
