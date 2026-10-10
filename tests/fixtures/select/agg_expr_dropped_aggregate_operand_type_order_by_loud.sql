-- The `ORDER BY` spelling of a dropped aggregate's operand type check, in a junk sort
-- key.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT a FROM o GROUP BY a ORDER BY true OR sum(t) > 0, a;
