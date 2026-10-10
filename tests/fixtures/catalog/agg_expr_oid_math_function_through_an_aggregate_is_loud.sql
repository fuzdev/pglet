-- A math function over an aggregate of an `oid`: `abs(max(conrelid))` is 18.6's
-- `42883 function abs(oid) does not exist`, the gate reached through the aggregate's
-- declared type as through a column's.
-- @expect-error: 42883 — abs over an `oid` produced by an aggregate
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT abs(max(conrelid)) FROM pg_constraint
