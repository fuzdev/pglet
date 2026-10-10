-- An expression key yielding a float, and partial indexes over a float key: all are
-- maintained on write, and a unique partial one lets a key repeat outside its predicate.
-- @profile-lock
-- @setup
CREATE TABLE fke (a INT, f FLOAT8);
CREATE INDEX fke_x ON fke ((f * 2));
CREATE UNIQUE INDEX fke_u ON fke ((a::float8 / 2));
CREATE INDEX fke_p ON fke (f) WHERE a > 0;
CREATE UNIQUE INDEX fke_pu ON fke (f) WHERE a < 0;
INSERT INTO fke VALUES (1, 1.5), (2, 1.5), (-1, 7), (-3, 8);
INSERT INTO fke VALUES (5, 7), (6, 7);
UPDATE fke SET f = f + 1 WHERE a = 6;
-- @test
SELECT a, f FROM fke ORDER BY a;
