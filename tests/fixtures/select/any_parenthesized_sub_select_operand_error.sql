-- The left operand of an `ANY ((SELECT …))` is evaluated ahead of pglet's refusal of the
-- sub-select form, so its division raises as it does on 18.6.
-- @setup
CREATE TABLE ta (x INT);
INSERT INTO ta VALUES (1);
-- @expect-error: division by zero
-- @test
SELECT x / 0 = ANY ((SELECT 1)) FROM ta;
