-- A CHECK constraint's operands are type-checked at CREATE TABLE (`42883`).
-- Accepting it turned every insert into an operator-resolution error instead of
-- either a check violation or a successful write.
-- @test
-- @expect-error: no integer ~~ text operator
CREATE TABLE ck_expr_ty (id INT CHECK (id LIKE '1%'));
