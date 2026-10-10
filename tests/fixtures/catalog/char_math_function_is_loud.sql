-- `abs(contype)` is `42883 function abs("char") does not exist` on 18.6.
-- @expect-error: 42883 — a math function over a `"char"`
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT abs(contype) FROM pg_constraint LIMIT 1
