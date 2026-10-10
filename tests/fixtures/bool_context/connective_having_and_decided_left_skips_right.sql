-- A grouping query stops at a deciding left operand too: `count(*) = 99` is `false` for
-- the group, so `1/(count(*)-2)` is never evaluated and the group is filtered out.
-- @setup
CREATE TABLE sa (id INT, g INT);
INSERT INTO sa VALUES (1, 1), (2, 1);
-- @test
SELECT g FROM sa GROUP BY g HAVING count(*) = 99 AND 1/(count(*)-2) = 0;
