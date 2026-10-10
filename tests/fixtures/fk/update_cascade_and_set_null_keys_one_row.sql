-- An `ON UPDATE CASCADE` key and an `ON UPDATE SET NULL` key reaching one row from one
-- parent row each take their action.
-- @setup
CREATE TABLE p (k text PRIMARY KEY);
INSERT INTO p VALUES ('a');
CREATE TABLE c (id int, k1 text REFERENCES p ON UPDATE CASCADE, k2 text REFERENCES p ON UPDATE SET NULL);
INSERT INTO c VALUES (1, 'a', 'a');
UPDATE p SET k = 'b';
-- @test
-- @profile-lock
SELECT id, k1, k2 FROM c;
