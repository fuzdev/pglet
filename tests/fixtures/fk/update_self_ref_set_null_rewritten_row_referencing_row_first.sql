-- @no-txn
-- Its own statements, not the harness's wrapper, write the rows: 18.6 checks the key of a
-- row its own transaction wrote even where the `UPDATE` leaves the key unchanged, and here
-- that check runs before the cascade reaches the row, so inside one transaction it refuses.
--
-- A row the statement rewrites that still references a key another rewritten row moves takes
-- the `ON UPDATE SET NULL` action, though it is written before the row it references.
-- @setup
DROP TABLE IF EXISTS sr_set_null_ing;
CREATE TABLE sr_set_null_ing (id int PRIMARY KEY, par int REFERENCES sr_set_null_ing ON UPDATE SET NULL);
INSERT INTO sr_set_null_ing VALUES (1, NULL), (3, NULL), (2, 1);
UPDATE sr_set_null_ing SET par = 2 WHERE id = 3;
-- @test
-- @profile-lock
UPDATE sr_set_null_ing SET id = id + 10 WHERE id IN (2, 3);
SELECT id, par FROM sr_set_null_ing ORDER BY id;
