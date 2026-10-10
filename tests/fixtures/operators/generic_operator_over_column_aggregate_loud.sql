-- An unknown operator over an aggregate of a column is refused `42883` like any other
-- operand: the refusal's message reads the operand's type before the coercion pass has
-- run, and must not ask the aggregate for what that pass records.
-- @setup
CREATE TABLE ou (a INT, b INT);
INSERT INTO ou VALUES (1, 10), (2, 0);
-- @test
-- @expect-error: operator does not exist
SELECT max(a) ! 1 FROM ou;
