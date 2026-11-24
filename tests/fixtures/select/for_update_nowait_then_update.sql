-- `FOR UPDATE NOWAIT` then an `UPDATE` of the row it read, in one block: the read takes
-- the writer as a plain `FOR UPDATE` does, and the block's write follows it.
-- @profile-lock
-- @setup
CREATE TABLE lnowait (id INT PRIMARY KEY, code TEXT);
INSERT INTO lnowait VALUES (1, 'a'), (2, 'b');
-- @test
SELECT id FROM lnowait WHERE id = 1 FOR UPDATE NOWAIT;
UPDATE lnowait SET code = 'z' WHERE id = 1;
SELECT id, code FROM lnowait ORDER BY id;
