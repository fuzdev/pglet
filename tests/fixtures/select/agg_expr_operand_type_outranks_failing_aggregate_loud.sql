-- The operand type check is raised ahead of every group's aggregates, as 18.6 raises
-- it in parse analysis: the `sum(1/b)` before it would divide by the `b = 0` row
-- (`22012`), and the statement is `42883`.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT sum(1/b), true OR sum(t) > 0 FROM o;
