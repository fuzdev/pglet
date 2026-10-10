-- `DROP SEQUENCE` frees the name: the re-`CREATE` would be 42P07 against a drop that
-- did nothing, and the counter it hands out is the *new* `START WITH` rather than the
-- old sequence's.
-- @setup
CREATE SEQUENCE dseq START WITH 5;
SELECT nextval('dseq');
DROP SEQUENCE dseq;
CREATE SEQUENCE dseq START WITH 90;
-- @test
SELECT nextval('dseq');
