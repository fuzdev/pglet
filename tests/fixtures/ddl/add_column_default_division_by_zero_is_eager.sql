-- **An `ADD COLUMN` evaluates its default; the other two routes only store it.** 18.6
-- computes the added column's `attmissingval` once at the `ALTER`
-- (`StoreAttrMissingVal`), so the value-dependent family is a DDL error on this route
-- — and over an **empty** table too, which is what separates it from a backfill
-- failing per row. Measured on 18.6: `22012` here, accepted on `CREATE TABLE` and on
-- `SET DEFAULT` (the two controls beside this file).
-- @setup
CREATE TABLE addcol_div (a INT);
-- @expect-error: an ADD COLUMN default is evaluated at DDL, so 1/0 is this statement's
-- @test
ALTER TABLE addcol_div ADD COLUMN b INT DEFAULT 1/0;
