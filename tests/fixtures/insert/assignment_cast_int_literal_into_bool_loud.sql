-- The headline of the assignment-cast gate. `int4 ↔ bool` is `pg_cast` context `'e'`
-- (explicit-only), so 18.6 answers `42804` in parse analysis while `SELECT 1::bool`
-- still answers `t`. pglet performed the conversion silently and **stored `true`**: the
-- planner wrapped every assignment right-hand side in an `Expr::Cast` to the column
-- type, so the evaluator converted under explicit-cast rules and the store finalizer —
-- which already refused this pair — never saw an integer at all.
-- @setup
CREATE TABLE aci (id INT, b BOOL);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
INSERT INTO aci (id, b) VALUES (1, 1);
