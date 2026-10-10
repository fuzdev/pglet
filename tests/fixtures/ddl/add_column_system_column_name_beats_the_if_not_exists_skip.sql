-- `check_for_column_name_collision` decides between two sentences and only one of them
-- has an `if_not_exists` arm: the *already exists* half skips, the system-column half
-- raises with no such test in the path. So the idempotent-migration spelling is a
-- refusal here and a **success** for an ordinary repeated column
-- (`types/typmod_add_column_if_not_exists_skips.sql` is that half). Measured on 18.6.
-- @setup
CREATE TABLE acsk (a INT);
-- @expect-error: the system column name is raised through the IF NOT EXISTS spelling
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE acsk ADD COLUMN IF NOT EXISTS ctid INT;
