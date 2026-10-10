-- Every row of a self-referencing hierarchy moves its key, and each child is repointed to
-- its parent's new key.
-- @setup
CREATE TABLE h (id int PRIMARY KEY, par int REFERENCES h ON UPDATE CASCADE);
INSERT INTO h VALUES (1, NULL), (2, 1), (3, 2);
UPDATE h SET id = id + 10;
-- @test
-- @profile-lock
SELECT id, par FROM h ORDER BY id;
