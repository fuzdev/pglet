-- The gate the direct spelling closed (`catalog/oid_arithmetic_is_loud.sql`) reads the
-- operand's **declared** type, and the reader it read it through could not see one
-- through the aggregate that produced it: `max(conrelid) + 1` answered 16385 where 18.6
-- raises `42883 operator does not exist: oid + integer`. Both walkers already declared
-- this column 26 — the gap was the gate's reader, not the information.
--
-- It is a **second** eval site as well as a second reader: an `Expr::Aggregate` never
-- reaches `eval_expr_full`'s `BinaryOp` arm at all (that arm refuses one outright), so
-- `executor::query::aggregate`'s own walk had to call the rulebook too.
-- @expect-error: 42883 — arithmetic over an `oid` produced by an aggregate
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT max(conrelid) + 1 FROM pg_constraint
