-- An `ON UPDATE CASCADE` into a `varchar(3)` child truncates an over-length key whose excess
-- is blanks, as an assignment does: `'ab   '` is stored as `'ab '`, which the parent also
-- holds, so the child's own key check passes. Unconverted, the child held `'ab   '`.
-- @setup
CREATE TABLE p (k varchar(10) PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk varchar(3) REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES ('a'), ('ab ');
INSERT INTO c VALUES (1, 'a');
UPDATE p SET k = 'ab   ' WHERE k = 'a';
-- @test
-- @profile-lock
SELECT id, '[' || pk || ']' AS pk, length(pk) AS len FROM c;
