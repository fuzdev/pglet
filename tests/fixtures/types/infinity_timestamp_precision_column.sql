-- A `timestamp(0)` column stores an infinity unrounded beside a rounded finite value.
-- @setup
CREATE TABLE infprec (ts TIMESTAMP(0), tz TIMESTAMPTZ(3));
INSERT INTO infprec VALUES ('infinity', '-infinity'), ('2000-01-01 00:00:00.7', '2000-01-01 00:00:00.0005+00');
-- @test
SELECT ts, tz FROM infprec ORDER BY ts;
