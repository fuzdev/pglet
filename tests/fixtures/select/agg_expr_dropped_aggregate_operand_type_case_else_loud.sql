-- The `ELSE` after a `CASE` arm a certain condition takes is dropped, and its
-- aggregate's operand type is still checked.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT CASE WHEN true THEN 'x' ELSE max(j)::text END FROM o;
