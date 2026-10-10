-- `IS DISTINCT FROM` resolves through the `=` operator, so a type-mismatched pair
-- is `42883` and Postgres names `=` in the message. Ungated it answered a
-- confident `true` for every row — worse than the plain `=` case, which at least
-- returned an unmatched `false`.
-- @setup
CREATE TABLE t (name TEXT);
INSERT INTO t VALUES ('5');
-- @test
-- @expect-error: no text = integer operator (via IS DISTINCT FROM)
SELECT name IS DISTINCT FROM 5 FROM t;
