-- Two `ON DELETE SET NULL` keys null one row twice, the second over the first, so the
-- row does not collide with its own first image under `UNIQUE (pad)`.
-- @setup
CREATE TABLE p (k text PRIMARY KEY);
INSERT INTO p VALUES ('a');
CREATE TABLE c (pad text UNIQUE, k1 text REFERENCES p ON DELETE SET NULL, k2 text REFERENCES p ON DELETE SET NULL);
INSERT INTO c VALUES ('p1', 'a', 'a');
DELETE FROM p;
-- @test
-- @profile-lock
SELECT pad, k1, k2 FROM c;
