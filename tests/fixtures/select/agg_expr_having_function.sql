-- `HAVING` over a function of an aggregate.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT a FROM u GROUP BY a HAVING round(avg(b)) > 1 ORDER BY a;
