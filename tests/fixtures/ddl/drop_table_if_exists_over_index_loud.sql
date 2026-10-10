-- The kind is decided on a name that *resolves*, so only a name no relation holds at
-- all reaches `DropErrorMsgNonExistent`'s skip. Read from the index side this is the
-- arrangement that made the class **silent**: pglet checked `sequences` alone, took the
-- skip, and reported success — after which `CREATE TABLE dtiei_ix` insisted the name
-- already existed, the exact contradiction the 42809 work exists to remove.
-- @setup
CREATE TABLE dtiei_t (a INT);
CREATE INDEX dtiei_ix ON dtiei_t (a);
-- @test
DROP TABLE IF EXISTS dtiei_ix;
-- @expect-error: an index is not a table, `IF EXISTS` notwithstanding (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
