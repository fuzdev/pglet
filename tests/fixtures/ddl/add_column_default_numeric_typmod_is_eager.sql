-- The `NUMERIC(p,s)` member of the family. `22003` again but through a different
-- mechanism from `add_column_default_overflow_is_eager.sql` — a precision overflow at
-- the typmod rather than an arithmetic one — and accepted on `CREATE TABLE` on both
-- engines, where it raises per row.
-- @setup
CREATE TABLE addcol_num (a INT);
-- @expect-error: an ADD COLUMN default is evaluated at DDL, so the numeric typmod is this statement's
-- @test
ALTER TABLE addcol_num ADD COLUMN b NUMERIC(4,2) DEFAULT 12345.6;
