-- `OR` in a grouping query's `HAVING`: `count(*) = 2` is `true` for the group, so the
-- division is never evaluated and the group is kept.
-- @setup
CREATE TABLE sa (id INT, g INT);
INSERT INTO sa VALUES (1, 1), (2, 1);
-- @test
SELECT g FROM sa GROUP BY g HAVING count(*) = 2 OR 1/(count(*)-2) = 0;
