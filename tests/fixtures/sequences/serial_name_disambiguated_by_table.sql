-- It is the *relation* namespace, not a sequence namespace: a table named
-- `<t>_<c>_seq` pushes the SERIAL's sequence to `<t>_<c>_seq1` exactly as a sequence
-- does. The test names that sequence explicitly — the inserted id alone is `1` either
-- way, since a shared name would land on a *fresh* sequence in the other map.
-- @setup
CREATE TABLE tabdis_a_seq (z INT);
INSERT INTO tabdis_a_seq VALUES (42);
CREATE TABLE tabdis (a SERIAL);
INSERT INTO tabdis (a) VALUES (DEFAULT);
-- @test
SELECT (SELECT a FROM tabdis) AS id,
       (SELECT z FROM tabdis_a_seq) AS z,
       nextval('tabdis_a_seq1') AS next_from_the_columns_sequence;
