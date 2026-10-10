-- Two `infinity` rows are one group and one distinct value, apart from `-infinity`.
-- @setup
CREATE TABLE infgd (d DATE, ts TIMESTAMP);
INSERT INTO infgd VALUES ('infinity', 'infinity'), ('infinity', '-infinity'), ('-infinity', '-infinity'), ('2000-01-01', 'infinity');
-- @test
SELECT d, count(*) AS n, count(DISTINCT ts) AS nts FROM infgd GROUP BY d ORDER BY d;
