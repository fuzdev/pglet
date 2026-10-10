-- An `ON UPDATE CASCADE` converts the parent's `int` key to the `int2` child column, so
-- `40000` is refused `22003`. Unconverted, the `int2` column held `40000`.
-- @setup
CREATE TABLE p (k int PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk int2 REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES (1);
INSERT INTO c VALUES (1, 1);
-- @test
-- @profile-lock
-- @expect-error: smallint out of range
UPDATE p SET k = 40000;
