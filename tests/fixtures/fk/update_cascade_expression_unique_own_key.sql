-- The control: a repointed child whose `lower(k)` is its own old one does not collide
-- with itself.
-- @setup
CREATE TABLE p (k text PRIMARY KEY);
CREATE TABLE c (id int, k text REFERENCES p ON UPDATE CASCADE);
CREATE UNIQUE INDEX c_u ON c (lower(k));
INSERT INTO p VALUES ('A'), ('b');
INSERT INTO c VALUES (1, 'A'), (2, 'b');
UPDATE p SET k = 'a' WHERE k = 'A';
-- @test
-- @profile-lock
SELECT id, k FROM c ORDER BY id;
