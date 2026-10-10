-- A `COALESCE` that stops at a `date` constant is still no constant where its type is
-- `timestamptz`: 18.6 brings the `date` to that type by a stable cast, so the condition
-- is not folded and the `OR` keeps its right operand's aggregate.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT coalesce('2020-01-01'::date, now()) IS NOT NULL OR sum(1/b) > 0 FROM u;
