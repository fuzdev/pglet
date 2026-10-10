-- A self-referencing table with two keys, each referenced `ON UPDATE CASCADE` by its own
-- column: every row moves both keys, and every reference follows.
-- @setup
CREATE TABLE s (a int UNIQUE, b int UNIQUE, fa int REFERENCES s (a) ON UPDATE CASCADE, fb int REFERENCES s (b) ON UPDATE CASCADE);
INSERT INTO s VALUES (1, 10, NULL, NULL), (2, 20, 1, 10), (3, 30, 2, 20);
-- @test
-- @profile-lock
UPDATE s SET a = a + 100, b = b + 100;
SELECT a, b, fa, fb FROM s ORDER BY a;
