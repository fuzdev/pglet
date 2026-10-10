-- `= ANY` between a `date` and a `timestamptz` array compares through a stable
-- cross-type operator on 18.6, so the condition is no constant and the `OR` keeps its
-- right operand's aggregate.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT '2020-01-01'::date = ANY (ARRAY['2020-01-01 00:00+00'::timestamptz]) OR sum(1/b) > 0 FROM u;
