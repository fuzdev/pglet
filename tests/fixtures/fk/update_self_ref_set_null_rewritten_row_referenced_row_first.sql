-- A row the statement rewrites that still references a key another rewritten row
-- moves takes the `ON UPDATE SET NULL` action, whichever of the two is written first.
-- @setup
CREATE TABLE p (id int PRIMARY KEY, par int REFERENCES p ON UPDATE SET NULL);
INSERT INTO p VALUES (1, NULL), (2, 1), (3, NULL);
UPDATE p SET par = 2 WHERE id = 3;
UPDATE p SET id = id + 10 WHERE id IN (2, 3);
-- @test
-- @profile-lock
SELECT id, par FROM p ORDER BY id;
