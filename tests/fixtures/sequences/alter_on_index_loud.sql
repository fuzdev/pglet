-- ...and an index target is 42809 as well.
-- @profile-lock
-- @setup
CREATE TABLE alter_idx_host (a INT);
CREATE INDEX alter_idx_target ON alter_idx_host (a);
-- @test
ALTER SEQUENCE alter_idx_target RESTART;
-- @expect-error: cannot open relation, not a sequence (42809)
