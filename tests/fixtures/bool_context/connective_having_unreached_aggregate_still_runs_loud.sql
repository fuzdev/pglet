-- An aggregate under the operand a decided `AND` skips still runs: 18.6 advances every
-- aggregate over the group before evaluating any expression above one, so the division
-- inside `sum(…)` raises whatever `count(*) = 99` answers.
-- @setup
CREATE TABLE sa (id INT, g INT);
INSERT INTO sa VALUES (1, 1), (2, 1);
-- @expect-error: division by zero
-- @test
SELECT g FROM sa GROUP BY g HAVING count(*) = 99 AND sum(1/(id-1)) > 0;
