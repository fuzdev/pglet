-- ALTER TABLE … RENAME is unsupported in pglet (fails loud 0A000) rather than the
-- old silent no-op that reported success while renaming nothing. Postgres renames,
-- so this is a deliberately pglet-stricter case — any pglet error passes.
-- @expect-error: pglet rejects ALTER TABLE RENAME
-- @setup
CREATE TABLE alter_rename_src (a INT);

-- @test
ALTER TABLE alter_rename_src RENAME TO alter_rename_dst;
