-- The same order without a size: a default two rows share, which also fails the CHECK, is
-- the build's `23505`, not the CHECK's `23514`.
-- @setup
CREATE TABLE t (id int);
INSERT INTO t VALUES (1), (2);
-- @test
-- @expect-error: the duplicate before the check
-- @expect-message
-- @profile-lock
ALTER TABLE t ADD COLUMN b int UNIQUE CHECK (b < 0) DEFAULT 5;
