-- The `ALTER TABLE … ADD CONSTRAINT … CHECK` path type-checks too. The existing-row
-- scan it already ran proves nothing on an empty table, so an un-typed predicate
-- would slip through and break every future insert.
-- @setup
CREATE TABLE alter_ck_ty (id INT);
-- @test
-- @expect-error: no integer ~~ text operator
ALTER TABLE alter_ck_ty ADD CONSTRAINT alter_ck_ty_c CHECK (id LIKE '1%');
