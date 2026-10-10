-- An `ON UPDATE CASCADE` applies the `numeric(5,2)` child column's modifier to the parent's
-- new key: `1.234` is stored as `1.23`, which the parent also holds, so the child's own key
-- check passes. Unconverted, the child held `1.234`.
-- @setup
CREATE TABLE p (k numeric PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk numeric(5,2) REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES (1), (1.23);
INSERT INTO c VALUES (1, 1);
UPDATE p SET k = 1.234 WHERE k = 1;
-- @test
-- @profile-lock
SELECT id, pk FROM c;
