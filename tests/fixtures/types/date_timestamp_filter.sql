-- A DATE column filtered against a TIMESTAMP threshold (explicit `::timestamp` cast,
-- so a mixed Date/Timestamp pair reaches the comparator) plus MIN/MAX over the
-- survivors — the date promotes to midnight UTC, so only rows past midday of Jan 1
-- survive (Jan 2 and Jan 3), and MIN/MAX return dates.
-- @setup
CREATE TABLE dt (id INT, d DATE);
INSERT INTO dt VALUES (1, '2024-01-01'), (2, '2024-01-02'), (3, '2024-01-03');
-- @test
SELECT min(d) AS lo, max(d) AS hi
FROM dt
WHERE d > '2024-01-01 12:00:00'::timestamp;
