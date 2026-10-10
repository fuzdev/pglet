-- An `ON UPDATE CASCADE` stores the parent's new key converted to the `bigint` child
-- column, so an index probe on that column finds the cascaded row. Stored as an `int4`, the
-- row was invisible to the `int8` index and the probe answered `0`.
-- @setup
CREATE TABLE p (k int PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk bigint REFERENCES p ON UPDATE CASCADE);
CREATE INDEX c_pk ON c (pk);
INSERT INTO p VALUES (1);
INSERT INTO c VALUES (1, 1);
UPDATE p SET k = 5;
-- @test
-- @profile-lock
SELECT count(*) FROM c WHERE pk = 5;
