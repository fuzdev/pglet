-- `-(SELECT contype …)` is `42883 operator does not exist: - "char"`, on 18.6 and here:
-- the negation gate reads the scalar subquery's declared `"char"`. A gate blind to the
-- declared type answers `0A000` ("negation not supported for type") instead.
-- @expect-error: 42883 — negation of a `"char"` produced by a scalar subquery
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT -(SELECT contype FROM pg_constraint LIMIT 1)
