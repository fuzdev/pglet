-- An `ON UPDATE CASCADE` writes the parent's new key into the child through the child
-- column's assignment conversion, as 18.6's action `UPDATE` does: a `bigint` child of an
-- `int` key stores an `int8`, so its `UNIQUE` index sees the cascaded `5` and refuses a
-- second `5`. Stored unconverted, the cascaded `int4` was invisible to the `int8` index and
-- the duplicate was accepted.
-- @setup
CREATE TABLE p (k int PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk bigint UNIQUE REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES (1), (7);
INSERT INTO c VALUES (1, 1);
UPDATE p SET k = 5 WHERE k = 1;
-- @test
-- @profile-lock
-- @expect-error: duplicate key value violates unique constraint "c_pk_key"
INSERT INTO c VALUES (2, 5);
