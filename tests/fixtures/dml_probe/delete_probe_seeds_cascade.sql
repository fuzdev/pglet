-- A primary-key-probed DELETE seeds the ON DELETE CASCADE: the children of the deleted
-- parent go with it, the others stay.
-- @profile-lock
-- @setup
CREATE TABLE cp (id INT PRIMARY KEY);
CREATE TABLE cc (id INT, pid INT REFERENCES cp (id) ON DELETE CASCADE);
INSERT INTO cp VALUES (1), (2), (3);
INSERT INTO cc VALUES (10, 2), (11, 1), (12, 2), (13, 3);
-- @test
DELETE FROM cp WHERE id = 2;
DELETE FROM cp WHERE id >= 3;
SELECT id, pid FROM cc ORDER BY id;
