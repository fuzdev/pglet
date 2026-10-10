-- An `INSERT … VALUES` item `NULL IN (1, (SELECT …))` stores NULL.
-- @setup
CREATE TABLE u (id INT PRIMARY KEY, v INT);
-- @profile-lock
-- @test
INSERT INTO u VALUES (3, (NULL::int IN (1, (SELECT 1)))::int);
SELECT id, v FROM u ORDER BY id;
