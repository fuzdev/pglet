-- A `SKIP LOCKED` claim then an `UPDATE` of the claimed row, in one block: the claim
-- takes the writer, and the block's write follows it.
-- @profile-lock
-- @setup
CREATE TABLE lskip (id INT PRIMARY KEY, code TEXT);
INSERT INTO lskip VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
SELECT id FROM lskip ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED;
UPDATE lskip SET code = 'claimed' WHERE id = 1;
SELECT id, code FROM lskip ORDER BY id FOR SHARE SKIP LOCKED;
