-- An `ON UPDATE CASCADE` rounds the parent's new key `1.234` to the `numeric(5,2)` child
-- column's `1.23`, which no parent row holds, so the rewritten child's own key check refuses.
-- Unconverted, the child held `1.234` and the statement ran.
-- @setup
CREATE TABLE p (k numeric PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk numeric(5,2) REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES (1);
INSERT INTO c VALUES (1, 1);
-- @test
-- @profile-lock
-- @expect-error: insert or update on table "c" violates foreign key constraint "c_pk_fkey"
UPDATE p SET k = 1.234;
