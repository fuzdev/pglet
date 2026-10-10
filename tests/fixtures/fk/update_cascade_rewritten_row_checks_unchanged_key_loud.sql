-- A row an action rewrites checks every one of its keys when the statement wrote the row
-- before, changed or not. The second row sets `q` to a key `qq` does not hold; its own
-- check is skipped because the first row's `ON UPDATE CASCADE` rewrites it first, and the
-- rewrite's check of `q`, which the action left unchanged, refuses it.
-- @setup
CREATE TABLE qq (k int PRIMARY KEY);
INSERT INTO qq VALUES (1);
CREATE TABLE t (id int PRIMARY KEY, par int REFERENCES t ON UPDATE CASCADE, q int REFERENCES qq);
INSERT INTO t VALUES (1, NULL, 1), (2, 1, 1);
-- @test
-- @expect-error: the rewritten row's unchanged key is checked
-- @expect-message
-- @profile-lock
UPDATE t SET id = CASE id WHEN 1 THEN 10 ELSE id END, q = CASE id WHEN 2 THEN 99 ELSE q END;
