-- The same gate against the route's *eager* family, which no `CREATE TABLE`
-- arrangement can reach: `DEFAULT 1/0` is a DDL error only where the added column's
-- `attmissingval` is computed (`add_column_default_division_by_zero_is_eager.sql`), and
-- the system column name still outranks it — `42701`, not `22012`.
-- @setup
CREATE TABLE acsed (a INT);
-- @expect-error: the system column name outranks the eager default evaluation
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE acsed ADD COLUMN ctid INT DEFAULT 1/0;
