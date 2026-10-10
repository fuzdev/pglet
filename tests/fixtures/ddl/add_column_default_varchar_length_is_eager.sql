-- The `22001` member of the family, and the one that shows the *typmod* is applied
-- eagerly too rather than only the expression evaluated: a `varchar(n)` length is
-- applied after the assignment cast, so `CREATE TABLE t (b VARCHAR(3) DEFAULT
-- 'abcdef')` is accepted on both engines while the `ADD COLUMN` spelling is `22001`.
-- @setup
CREATE TABLE addcol_len (a INT);
-- @expect-error: an ADD COLUMN default is evaluated at DDL, so the varchar length is this statement's
-- @test
ALTER TABLE addcol_len ADD COLUMN b VARCHAR(3) DEFAULT 'abcdef';
