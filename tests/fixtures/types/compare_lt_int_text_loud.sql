-- The ordering operators are gated with `=`: `integer < text` has no operator
-- (`42883`). Ungated, `values_compare` fell through to a text-rendering
-- comparison, so the predicate silently answered in *text* order.
-- @setup
CREATE TABLE t (n INT, s TEXT);
INSERT INTO t VALUES (10, '9');
-- @test
-- @expect-error: no integer < text operator
SELECT * FROM t WHERE n < s;
