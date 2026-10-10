-- A function of an aggregate per group, beside a grouped column.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT a, abs(sum(b) - 5) AS d, upper(max(y)) AS up, round(avg(b), 1) AS r FROM u GROUP BY a ORDER BY a;
