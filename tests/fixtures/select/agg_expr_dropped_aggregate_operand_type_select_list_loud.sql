-- Dropping an aggregate drops its computation, not its operand's type check: 18.6
-- resolves `sum(text)` in parse analysis, before its planner folds the `OR` away, so the
-- statement is `42883` whatever the rows. Checking only the aggregates a group computes
-- would answer `t`.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT true OR sum(t) > 0 FROM o;
