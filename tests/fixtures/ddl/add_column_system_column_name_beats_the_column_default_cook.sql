-- The `ALTER TABLE … ADD COLUMN` half. `check_for_column_name_collision` runs ahead of
-- the `BuildDescForRelation` that resolves the column's type, so both names it answers
-- — a column the table already has and a system-owned one — are above the cook on this
-- route as well.
-- @setup
CREATE TABLE acsdc (a INT);
-- @expect-error: the system column name outranks the column default cook
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE acsdc ADD COLUMN ctid INT DEFAULT 'zz';
