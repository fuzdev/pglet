-- A column constraint takes no `NOT VALID`: `42601`.
-- @profile-lock
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @expect-error: syntax error at or near "VALID"
-- @test
CREATE TABLE nv3 (a int CHECK (a > 0) NOT VALID);
