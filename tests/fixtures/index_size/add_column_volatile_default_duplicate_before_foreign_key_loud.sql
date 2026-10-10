-- The foreign key is checked after the rewrite and the build: with no parent row, the
-- per-row default's repeated 5 is the build's `23505`, not the key's `23503`.
-- @setup
CREATE TABLE p (k int PRIMARY KEY);
CREATE TABLE t (id int);
INSERT INTO t VALUES (1), (2);
-- @test
-- @expect-error: the build's duplicate before the foreign key
-- @expect-message
-- @profile-lock
ALTER TABLE t ADD COLUMN b int UNIQUE REFERENCES p DEFAULT (length(gen_random_uuid()::text) - 31);
