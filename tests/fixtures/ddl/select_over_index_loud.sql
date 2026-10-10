-- `validate_relation_kind` refuses a scan of an index (`cannot open relation "x"`,
-- 42809). Unlike a sequence — which Postgres *does* let you read — nothing about this
-- direction is a pglet divergence: both engines refuse, and pglet used to refuse with
-- the wrong code.
-- @setup
CREATE TABLE soi_t (a INT);
CREATE INDEX soi_ix ON soi_t (a);
-- @test
SELECT * FROM soi_ix;
-- @expect-error: an index is not a readable relation (42809)
-- @no-profile-lock: DDL-shaped `@test` — the target never resolves, so no index selection runs
