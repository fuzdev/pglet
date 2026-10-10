-- `ADD COLUMN` puts the name collision **ahead** of the type, where `CREATE TABLE` puts
-- the type first — 18.6's own asymmetry, which `check_for_column_name_collision` running
-- before `BuildDescForRelation` produces and which pglet mirrors rather than normalises.
-- So the system-column half of that collision outranks a refused type modifier here,
-- where the `CREATE TABLE` spelling of the same pair is `22023`
-- (`column_type_beats_the_system_column_name.sql`).
-- @setup
CREATE TABLE acst (a INT);
-- @expect-error: the system column name outranks the refused char length
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE acst ADD COLUMN ctid CHAR(0);
