-- A `SERIAL` column's sequence name is chosen by the same `ChooseRelationName` an
-- unnamed index's is, so it is cut to 63 bytes the same way: the 40-byte table and column
-- names both give up bytes and the `seq` label is kept whole. The column's default and
-- `pg_get_serial_sequence` name the sequence that was created, and the column draws from it.
-- @setup
CREATE TABLE sln1_ttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo SERIAL, v INT);
INSERT INTO sln1_ttttttttttttttttttttttttttttttttttt (v) VALUES (10), (20);
-- @test
SELECT pg_get_serial_sequence('sln1_ttttttttttttttttttttttttttttttttttt', 'cooooooooooooooooooooooooooooooooooooooo') AS seq,
       length(pg_get_serial_sequence('sln1_ttttttttttttttttttttttttttttttttttt', 'cooooooooooooooooooooooooooooooooooooooo')) AS len,
       max(cooooooooooooooooooooooooooooooooooooooo) AS drawn
FROM sln1_ttttttttttttttttttttttttttttttttttt;
