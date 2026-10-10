-- A kept aggregate over a cast is not judged by the cast's target ahead of its rows: the
-- cast is itself refused (`42846`, as 18.6 refuses it before it resolves the
-- aggregate), where `max(jsonb)` alone would be `42883`.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT max(a::jsonb) FROM o;
