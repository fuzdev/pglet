-- An `ON UPDATE SET NULL` into a NOT NULL child column, moving a key no child row
-- references: NOT NULL is a check of each row the action writes, and the action writes
-- none, so the `UPDATE` stands. The action converts the value it assigns ahead of the lookup
-- of the rows it rewrites, and a NOT NULL refusal made there would refuse this statement.
-- With a referencing row the NOT NULL refuses (`fk/on_update_set_null_not_null.sql`).
-- @setup
CREATE TABLE p (k int PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk int NOT NULL REFERENCES p ON UPDATE SET NULL);
INSERT INTO p VALUES (1), (2);
INSERT INTO c VALUES (1, 2);
-- @test
-- @profile-lock
UPDATE p SET k = 10 WHERE k = 1;
SELECT k FROM p ORDER BY k;
