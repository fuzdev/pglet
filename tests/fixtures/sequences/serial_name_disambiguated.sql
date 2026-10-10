-- A SERIAL column's sequence name is the *engine's*, so a taken `<table>_<col>_seq`
-- is disambiguated (t_a_seq -> t_a_seq1) rather than shared. Reading the inserted id
-- is what distinguishes the two: the pre-existing sequence has been advanced twice,
-- so a shared name would hand the row a 3.
-- @setup
CREATE SEQUENCE serdis_a_seq;
SELECT nextval('serdis_a_seq');
SELECT nextval('serdis_a_seq');
CREATE TABLE serdis (a SERIAL, b INT);
INSERT INTO serdis (b) VALUES (10);
-- @test
SELECT a, b FROM serdis;
