-- A grouping query's select-list spelling of both connectives.
-- @setup
CREATE TABLE sa (id INT, g INT);
INSERT INTO sa VALUES (1, 1), (2, 1);
-- @test
SELECT g, (count(*) = 99 AND 1/(count(*)-2) = 0) AS a, (count(*) = 2 OR 1/(count(*)-2) = 0) AS o
FROM sa GROUP BY g;
