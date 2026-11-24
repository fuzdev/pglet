-- An aggregate over a no-FROM select's row reads one row or none: `count(*)` is 0 under a
-- false `WHERE`, and a `HAVING` filters the one group.
-- @test
SELECT (SELECT count(*) WHERE false) AS none, (SELECT count(*) WHERE true) AS one,
       (SELECT 7 HAVING false) AS dropped, (SELECT 8 GROUP BY 1 HAVING count(*) = 1) AS kept;
