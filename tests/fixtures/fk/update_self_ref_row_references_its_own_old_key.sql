-- A row's actions run before its own key check: the row moves its key from `1` to `2` and
-- newly references `1`, its own old key; its `ON UPDATE CASCADE` repoints it to `2`, and
-- the check of the version the action superseded is skipped.
-- @setup
CREATE TABLE t (id int PRIMARY KEY, par int REFERENCES t ON UPDATE CASCADE);
INSERT INTO t VALUES (1, NULL);
-- @test
-- @profile-lock
UPDATE t SET id = 2, par = 1;
SELECT id, par FROM t;
