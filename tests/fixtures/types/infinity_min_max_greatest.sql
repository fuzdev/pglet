-- `min` / `max` and `greatest` / `least` pick an infinity over the finite values beside it,
-- the domain's edges among them.
-- @setup
CREATE TABLE infmm (d DATE, ts TIMESTAMP);
INSERT INTO infmm VALUES ('infinity', '2000-01-01'), ('2000-01-01', '-infinity'), ('-infinity', 'infinity');
-- @test
SELECT min(d) AS dmin, max(d) AS dmax, min(ts) AS tmin, max(ts) AS tmax,
       greatest(date 'infinity', date '9999-12-31') AS g, least(timestamp '-infinity', timestamp '0001-01-01') AS l
FROM infmm;
