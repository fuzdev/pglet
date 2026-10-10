-- `IF EXISTS` does NOT soften a wrong kind: only a name no relation holds at all
-- reaches `DropErrorMsgNonExistent`'s skip, so this is `42809` exactly as the bare
-- spelling is. Read the other way round the class goes silent — the statement would
-- report success and the next one to name `dwki_seq` would insist it already exists.
-- Its accepting mirror is `drop_table_multi_if_exists_skips_missing.sql`, where the
-- second name resolves to nothing at all.
-- @setup
CREATE TABLE dwki (a INT);
CREATE SEQUENCE dwki_seq;

-- @expect-error: "dwki_seq" is not a table
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE IF EXISTS dwki, dwki_seq;
