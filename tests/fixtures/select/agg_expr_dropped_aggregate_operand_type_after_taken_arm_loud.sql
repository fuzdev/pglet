-- What follows a `CASE` arm a certain condition takes is dropped, a later arm's
-- condition included, and an aggregate there still has its operand type checked.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT CASE WHEN true THEN 1 WHEN max(j) IS NULL THEN 2 END FROM o;
