-- `text = integer` resolves to no operator in Postgres (`42883`). pglet used to
-- dispatch `=` on the runtime `Value` variant with a permissive fallback, so a
-- mistyped predicate silently matched **zero rows** instead of failing loud.
-- (A bare *string* literal is a different case and stays accepted — see
-- `compare_int_bad_literal_loud.sql`; an `int` literal carries a real type.)
-- @setup
CREATE TABLE t (name TEXT);
INSERT INTO t VALUES ('5');
-- @test
-- @expect-error: no text = integer operator
SELECT * FROM t WHERE name = 5;
