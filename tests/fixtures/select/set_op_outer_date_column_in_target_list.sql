-- A scalar subquery's set operation over the enclosing `date` column is a `date` column,
-- and the constant converts to it.
-- @setup
CREATE TABLE sod (d DATE);
INSERT INTO sod VALUES ('2020-01-01');
-- @test
SELECT (SELECT d UNION ALL SELECT '2020-01-02' LIMIT 1) AS v,
       (SELECT count(*) FROM (SELECT d UNION SELECT '2020-1-1') s) AS n
FROM sod;
