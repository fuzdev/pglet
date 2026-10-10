-- The `timestamptz` twin: a bare `ORDER BY` over a primary key holding both infinities.
-- @profile-lock
-- @setup
CREATE TABLE infpkt (ts TIMESTAMPTZ PRIMARY KEY);
INSERT INTO infpkt VALUES ('infinity'), ('-infinity'), ('2000-01-01 00:00:00+00'), ('9999-12-31 23:59:59.999999+00');
-- @test
SELECT ts FROM infpkt ORDER BY ts;
