-- The other shape the four-arm declared-type reader could not see: a scalar subquery.
-- `(SELECT conrelid …) + 1` answered 16385 where 18.6 raises `42883 operator does not
-- exist: oid + integer`. The subquery leaf resolves through the catalog this evaluation
-- was lent, which is why it answers here and not in a context with none.
-- @expect-error: 42883 — arithmetic over an `oid` produced by a scalar subquery
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT (SELECT conrelid FROM pg_constraint LIMIT 1) + 1
