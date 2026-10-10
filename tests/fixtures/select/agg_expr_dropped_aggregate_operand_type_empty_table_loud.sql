-- A dropped aggregate's operand type is checked whatever the rows: over an empty table
-- too, `sum(text)` is `42883`.
-- @setup
CREATE TABLE e (a INT, t TEXT);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT true OR sum(t) > 0 FROM e;
