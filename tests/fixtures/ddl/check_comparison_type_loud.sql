-- A CHECK constraint's comparison operands are type-checked at CREATE TABLE
-- (`42883`), where Postgres reports it. Recording the constraint instead would
-- make the table unwritable: every insert re-evaluates the stored expression and
-- the eval-time gate would reject it there.
-- @test
-- @expect-error: no text = integer operator
CREATE TABLE ck_cmp_ty (name TEXT CHECK (name = 5));
