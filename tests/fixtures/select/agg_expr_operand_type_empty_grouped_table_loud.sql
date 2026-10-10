-- A grouped query over an empty table forms no group, and its aggregate's operand type
-- is refused all the same: `max(jsonb)` is `42883`.
-- @setup
CREATE TABLE e (a INT, j JSONB);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT a, max(j) FROM e GROUP BY a;
