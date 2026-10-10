-- An equality probe for each infinity finds exactly its own row.
-- @profile-lock
-- @setup
CREATE TABLE infpke (d DATE PRIMARY KEY, v INT);
INSERT INTO infpke VALUES ('infinity', 1), ('-infinity', 2), ('9999-12-31', 3);
-- @test
SELECT v FROM infpke WHERE d = date 'infinity' OR d = date '-infinity' ORDER BY v;
