-- `ATSimplePermissions` refuses every `ALTER TABLE` action over an index (42809). The
-- kind check is the target lookup's, so it covers all of them at once.
-- @setup
CREATE TABLE atdci_t (a INT, b INT);
CREATE INDEX atdci_ix ON atdci_t (a);
-- @test
ALTER TABLE atdci_ix DROP COLUMN a;
-- @expect-error: an index is not an ALTER TABLE target (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
