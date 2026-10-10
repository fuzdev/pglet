-- A repointed child row is probed under every enforced unique index, not only the
-- referenced key's: under `lower(k)` the repointed `B` collides with the untouched `b`.
-- @setup
CREATE TABLE p (k text PRIMARY KEY);
CREATE TABLE c (id int, k text REFERENCES p ON UPDATE CASCADE);
CREATE UNIQUE INDEX c_u ON c (lower(k));
INSERT INTO p VALUES ('A'), ('b');
INSERT INTO c VALUES (1, 'A'), (2, 'b');
-- @test
-- @expect-error: the repointed key duplicates another row under lower(k)
-- @expect-message
-- @profile-lock
UPDATE p SET k = 'B' WHERE k = 'A';
