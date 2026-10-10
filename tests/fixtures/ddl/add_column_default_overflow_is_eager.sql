-- The `22003` member of the eagerly-evaluated `ADD COLUMN` default family (see
-- `add_column_default_division_by_zero_is_eager.sql`). An arithmetic overflow on a
-- typed constant: 18.6 raises it at the `ALTER` and accepts the identical default on
-- `CREATE TABLE`.
-- @setup
CREATE TABLE addcol_ovf (a INT);
-- @expect-error: an ADD COLUMN default is evaluated at DDL, so the overflow is this statement's
-- @test
ALTER TABLE addcol_ovf ADD COLUMN b INT DEFAULT (2147483647 + 1);
