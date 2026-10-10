-- An ordered, bounded read over an indexed `timestamp` column holding both infinities and
-- a NULL.
-- @profile-lock
-- @setup
CREATE TABLE infio (ts TIMESTAMP);
CREATE INDEX infio_ts ON infio (ts);
INSERT INTO infio VALUES ('infinity'), ('2000-01-01 12:00:00'), ('-infinity'), (NULL), ('0001-01-01 00:00:00');
-- @test
SELECT ts FROM infio WHERE ts >= timestamp '-infinity' ORDER BY ts DESC;
