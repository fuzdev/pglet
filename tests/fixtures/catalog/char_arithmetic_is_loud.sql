-- `"char"` borrows `text`'s value shape the way `oid` borrows `int4`'s, so the same gate
-- covers it: `contype + 1` is `42883 operator does not exist: "char" + integer` on 18.6.
-- It was loud here before the declared-type gate, but under `eval_arithmetic`'s own
-- `42804` ("operator + requires numeric operands") — a wrong code, and one that would
-- have gone silent for any `"char"` value that parsed as a number.
-- @expect-error: 42883 — arithmetic over a `"char"`
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT contype + 1 FROM pg_constraint LIMIT 1
