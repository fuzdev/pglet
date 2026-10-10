-- The third membership: an index's name is in the same namespace, so a sequence
-- cannot take it either.
-- @profile-lock
-- @setup
CREATE TABLE ns_idx_host (a INT);
CREATE INDEX ns_idx_taken ON ns_idx_host (a);
-- @test
CREATE SEQUENCE ns_idx_taken;
-- @expect-error: relation already exists (42P07)
