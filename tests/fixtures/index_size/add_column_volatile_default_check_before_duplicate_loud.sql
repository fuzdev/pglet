-- The per-row default here is 5 in every row, so the build would find a duplicate — but the
-- rewrite's CHECK comes first, `23514`, where `DEFAULT 5` is the build's `23505`.
-- @setup
CREATE TABLE t (id int);
INSERT INTO t VALUES (1), (2);
-- @test
-- @expect-error: the rewrite's CHECK before the build's duplicate
-- @expect-message
-- @profile-lock
ALTER TABLE t ADD COLUMN b int UNIQUE CHECK (b < 0) DEFAULT (length(gen_random_uuid()::text) - 31);
