-- The orphan-cleanup idiom over an empty parent, autocommitted: both rows go, the NULL one
-- included, and the statement's count is read back after it commits.
-- @no-txn
-- @setup
DROP TABLE IF EXISTS f3_child;
DROP TABLE IF EXISTS f3_parent;
CREATE TABLE f3_parent (id INT);
CREATE TABLE f3_child (x INT);
INSERT INTO f3_child VALUES (1), (NULL);
-- @test
DELETE FROM f3_child WHERE x NOT IN (SELECT id FROM f3_parent);
SELECT count(*) FROM f3_child;
