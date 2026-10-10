-- The key-column resolution is parse analysis and the already-exists check is execution
-- (`heap_create_with_catalog`), so 42703 beats 42P07 on a table that is already there —
-- the same split that puts 42P16 ahead of 42P07 in
-- `create_table_two_pk_beats_other_errors.sql`.
--
-- `IF NOT EXISTS` still outranks it, because `transformCreateStmt` tests that at the top
-- of the very same pass (`create_table_if_not_exists_beats_key_column.sql`).
-- @expect-error: column nosuch named in key does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE ctke (a INT);

-- @test
CREATE TABLE ctke (a INT, PRIMARY KEY (nosuch));
