-- Without a `GROUP BY` an empty input is still one group, so a function of an aggregate
-- answers one row over no rows.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT abs(max(a)) AS m, coalesce(round(avg(a), 1), -1) AS r, concat('n=', count(*)) AS c
FROM u WHERE false;
