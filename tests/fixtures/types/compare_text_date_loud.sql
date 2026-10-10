-- `text = date` has no operator (`42883`) — the two are not in one family, and a
-- typed `date` operand (unlike a bare string literal) is not coerced to the text
-- column's type.
-- @setup
CREATE TABLE t (name TEXT);
INSERT INTO t VALUES ('2024-06-15');
-- @test
-- @expect-error: no text = date operator
SELECT * FROM t WHERE name = '2024-06-15'::date;
