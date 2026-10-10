-- An `ON UPDATE CASCADE` into a NOT NULL child column, moving a nullable referenced key to
-- NULL where no child row references the old key: NOT NULL is a check of each row the
-- action writes, and the action writes none, so the `UPDATE` stands. The action converts the
-- key it assigns ahead of the lookup of the rows it rewrites, and a NOT NULL refusal made
-- there would refuse this statement. With a referencing row the NOT NULL refuses
-- (`fk/update_cascade_null_into_not_null_loud.sql`).
-- @setup
CREATE TABLE p (id int PRIMARY KEY, k int UNIQUE);
CREATE TABLE c (id int PRIMARY KEY, pk int NOT NULL REFERENCES p (k) ON UPDATE CASCADE);
INSERT INTO p VALUES (1, 1), (2, 2);
INSERT INTO c VALUES (1, 2);
-- @test
-- @profile-lock
UPDATE p SET k = NULL WHERE id = 1;
SELECT id, k FROM p ORDER BY id;
