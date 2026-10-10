-- `HAVING` over a function of the exact `avg` of a `VALUES` column keeps the group 18.6
-- keeps: the average `2.5` rounds to `3`.
-- @setup
CREATE TABLE o (id INT, a INT);
INSERT INTO o VALUES (1, 1), (2, 1), (3, 2);
-- @test
SELECT a, count(*) AS n, avg(x) AS av FROM o JOIN (VALUES (1), (4)) v(x) ON true
GROUP BY a HAVING round(avg(x)) > 2 ORDER BY a;
