-- An aggregate's operand type is checked for the statement, not per group: `HAVING`
-- keeps no group here, and `sum(text)` is `42883` all the same.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT sum(t) FROM o GROUP BY a HAVING count(*) > 5;
