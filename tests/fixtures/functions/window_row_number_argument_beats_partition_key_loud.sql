-- The call resolves before the window's keys are bound: the argument's refusal outranks
-- the unknown `PARTITION BY` column.
-- @setup
CREATE TABLE wna (a INT);
-- @test
-- @expect-error: function row_number(integer) does not exist (42883)
SELECT row_number(1) OVER (PARTITION BY nosuch) FROM wna;
