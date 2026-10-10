-- A `COALESCE` argument after a number literal is dropped, and its aggregate's operand
-- type is still checked: `avg(text)` is `42883`.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT coalesce(1.5, avg(t)) FROM o;
