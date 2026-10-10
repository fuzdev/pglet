-- A row the `DELETE` removes by `ON DELETE CASCADE` no longer holds its unique key, so the
-- row an `ON DELETE SET NULL` key nulls onto that key does not collide with it.
-- @setup
CREATE TABLE p (k int PRIMARY KEY);
INSERT INTO p VALUES (1), (2);
CREATE TABLE c (id int, a int REFERENCES p ON DELETE SET NULL, b int REFERENCES p ON DELETE CASCADE, k int);
CREATE UNIQUE INDEX c_u ON c (a, k) NULLS NOT DISTINCT;
INSERT INTO c VALUES (1, 1, 1, 7), (2, 2, NULL, 7);
-- @test
-- @profile-lock
DELETE FROM p;
SELECT id, a, b, k FROM c ORDER BY id;
