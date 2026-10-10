-- The `ALTER TABLE … ADD COLUMN` half of the same list — 18.6 reads it through
-- `check_for_column_name_collision`, which answers this sentence when the name
-- resolves to a negative `attnum` and the *already exists* one otherwise. Three of the
-- six names are pinned on this route and three on `CREATE TABLE`; see the
-- `create_table_system_column_name_*` siblings.
-- @setup
CREATE TABLE acsc_tableoid (a INT);
-- @expect-error: tableoid is a system column name
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE acsc_tableoid ADD COLUMN tableoid INT;
