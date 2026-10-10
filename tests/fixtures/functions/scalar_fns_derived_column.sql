-- A derived table's column computed by one of the new functions is read outside with
-- the type the function gave it: compared, aggregated, and folded.
-- @setup
CREATE TABLE dx (id INT, t TEXT, a INT);
INSERT INTO dx VALUES (1, 'apple', 4), (2, 'avocado', 9), (3, 'banana', NULL);
-- @test
SELECT s.l, count(*) AS n, max(s.ol) AS max_ol, sum(s.m) AS sum_m, avg(s.p) AS avg_p
FROM (SELECT left(t, 1) AS l, octet_length(t) AS ol, mod(a, 4) AS m, power(a, 2) AS p FROM dx) s
WHERE s.l = 'a' OR s.ol > 5
GROUP BY s.l ORDER BY s.l;
