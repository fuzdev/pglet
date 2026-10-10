-- The aggregate route for `"char"`. Note the two engines refuse it under **different
-- operand names**: 18.6 resolves `max("char")` through the implicit `"char" → text` cast
-- and reports `text + integer`, while pglet keeps `"char"` and reports `"char" + integer`.
-- The SQLSTATE is what this fixture asserts; the declared-type divergence behind the
-- message is `EDGES.md` row 87.
-- @expect-error: 42883 — arithmetic over a `"char"` produced by an aggregate
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT max(contype) + 1 FROM pg_constraint
