-- The disambiguation counts up: with both `<t>_<c>_seq` and `<t>_<c>_seq1` taken the
-- column lands on `<t>_<c>_seq2`. Both pre-existing sequences are *advanced* first,
-- so a build that shared either of them hands the row a different id — reading a
-- fresh `1` would pass whether or not the name moved.
-- @setup
CREATE SEQUENCE serdis2_a_seq;
SELECT nextval('serdis2_a_seq');
SELECT nextval('serdis2_a_seq');
CREATE SEQUENCE serdis2_a_seq1 START WITH 100;
SELECT nextval('serdis2_a_seq1');
CREATE TABLE serdis2 (a SERIAL);
INSERT INTO serdis2 (a) VALUES (DEFAULT);
-- @test
SELECT (SELECT a FROM serdis2) AS id,
       nextval('serdis2_a_seq2') AS next_from_the_columns_sequence;
