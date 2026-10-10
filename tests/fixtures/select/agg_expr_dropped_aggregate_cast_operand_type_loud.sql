-- A dropped aggregate over a cast takes its verdict from the cast's target type, nothing
-- evaluating the argument: `sum(text)` is `42883`.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT true OR sum(a::text) > 0 FROM o;
