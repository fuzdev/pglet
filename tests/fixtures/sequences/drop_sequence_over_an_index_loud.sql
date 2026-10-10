-- The kind check reads the whole relation namespace, not `schema.sequences` plus
-- `schema.tables`: an index is exactly as wrong a `DROP SEQUENCE` target as a table.
-- @setup
CREATE TABLE dsoi_t (a INT);
CREATE INDEX dsoi_ix ON dsoi_t (a);
-- @test
DROP SEQUENCE dsoi_ix;
-- @expect-error: an index is not a sequence (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
