-- A `CASE` arm a certain condition drops still has its aggregate's operand type
-- checked: `min(boolean)` is `42883`.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT CASE WHEN false THEN min(bo) ELSE true END FROM o;
