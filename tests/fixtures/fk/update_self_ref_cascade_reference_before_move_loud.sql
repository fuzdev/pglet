-- A row's own key is checked after its own actions and before the next row's. The first
-- row newly references key `2`, which the second row moves; the first row's check runs
-- before the second row's `ON UPDATE CASCADE` action, finds `2` gone, and refuses `23503`.
-- @setup
CREATE TABLE t (id int PRIMARY KEY, par int REFERENCES t ON UPDATE CASCADE);
INSERT INTO t VALUES (1, NULL), (2, NULL);
-- @test
-- @expect-error: the first row's key is checked before the second row's action
-- @expect-message
-- @profile-lock
UPDATE t SET par = CASE id WHEN 1 THEN 2 ELSE par END, id = CASE id WHEN 2 THEN 3 ELSE id END;
