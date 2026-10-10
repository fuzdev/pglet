-- The unary half over an aggregate: `-max(conrelid)` answered -16384 where 18.6 raises
-- `42883 operator does not exist: - oid`.
-- @expect-error: 42883 — negation of an `oid` produced by an aggregate
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT -max(conrelid) FROM pg_constraint
