-- A partial index whose predicate excludes `infinity`, read through that predicate.
-- @profile-lock
-- @setup
CREATE TABLE infpi (d DATE, v INT);
CREATE INDEX infpi_d ON infpi (d) WHERE d < date 'infinity';
INSERT INTO infpi VALUES ('infinity', 1), ('2000-01-01', 2), ('-infinity', 3);
-- @test
SELECT v FROM infpi WHERE d < date 'infinity' ORDER BY d;
