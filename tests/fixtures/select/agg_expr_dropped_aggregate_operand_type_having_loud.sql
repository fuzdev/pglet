-- The `HAVING` spelling of a dropped aggregate's operand type check: `max(jsonb)` is
-- `42883` though the `false AND` drops it.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT a FROM o GROUP BY a HAVING false AND max(j) IS NULL;
