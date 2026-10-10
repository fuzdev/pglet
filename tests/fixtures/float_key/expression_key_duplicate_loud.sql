-- A unique expression key computing a float: `2 / 2` is taken by the row holding `2`.
-- @profile-lock
-- @expect-error: the computed key 1 is taken (23505)
-- @expect-message
-- @setup
CREATE TABLE fke (a INT, f FLOAT8);
CREATE INDEX fke_x ON fke ((f * 2));
CREATE UNIQUE INDEX fke_u ON fke ((a::float8 / 2));
CREATE INDEX fke_p ON fke (f) WHERE a > 0;
CREATE UNIQUE INDEX fke_pu ON fke (f) WHERE a < 0;
INSERT INTO fke VALUES (1, 1.5), (2, 1.5), (-1, 7), (-3, 8);
-- @test
INSERT INTO fke VALUES (2, 9);
