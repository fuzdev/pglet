-- The rows of `update_self_ref_cascade_reference_before_move_loud`, written the other way
-- round: the row that moves key `2` is written first, so its `ON UPDATE CASCADE` repoints
-- the other row before that row's check runs, and the check of the version the action
-- superseded is skipped.
-- @setup
CREATE TABLE t (id int PRIMARY KEY, par int REFERENCES t ON UPDATE CASCADE);
INSERT INTO t VALUES (2, NULL), (1, NULL);
-- @test
-- @profile-lock
UPDATE t SET par = CASE id WHEN 1 THEN 2 ELSE par END, id = CASE id WHEN 2 THEN 3 ELSE id END;
SELECT id, par FROM t ORDER BY id;
