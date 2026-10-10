-- An `INSERT … VALUES` item `NULL IN (SELECT …)` over a sub-select with rows stores NULL.
-- @setup
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1), (2);
CREATE TABLE u (id INT PRIMARY KEY, v INT);
-- @profile-lock
-- @test
INSERT INTO u VALUES (1, (NULL::int IN (SELECT a FROM r))::int);
SELECT id, v FROM u ORDER BY id;
