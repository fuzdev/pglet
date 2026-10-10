-- @no-txn
-- Its own statements, not the harness's wrapper, write the rows: 18.6 checks the key of a
-- row its own transaction wrote even where the `UPDATE` leaves the key unchanged, and here
-- that check runs before the cascade reaches the row, so inside one transaction it refuses.
--
-- A row the statement rewrites that still references a key another rewritten row moves takes
-- the `ON UPDATE CASCADE` action, though it is written before the row it references.
-- @setup
DROP TABLE IF EXISTS sr_cascade_ing;
CREATE TABLE sr_cascade_ing (id int PRIMARY KEY, par int REFERENCES sr_cascade_ing ON UPDATE CASCADE);
INSERT INTO sr_cascade_ing VALUES (1, NULL), (3, NULL), (2, 1);
UPDATE sr_cascade_ing SET par = 2 WHERE id = 3;
-- @test
-- @profile-lock
UPDATE sr_cascade_ing SET id = id + 10 WHERE id IN (2, 3);
SELECT id, par FROM sr_cascade_ing ORDER BY id;
