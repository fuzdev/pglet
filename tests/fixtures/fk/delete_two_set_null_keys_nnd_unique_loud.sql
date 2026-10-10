-- Two `ON DELETE SET NULL` keys null one row twice, the second over the first, and each
-- nulled image is compared under the unique index with the images already planned: the
-- first row ends `(NULL, NULL)`, and the second row's second nulling collides with it
-- under `NULLS NOT DISTINCT`.
-- @setup
CREATE TABLE p (k text PRIMARY KEY);
INSERT INTO p VALUES ('a'), ('b');
CREATE TABLE c (id int, k1 text REFERENCES p ON DELETE SET NULL, k2 text REFERENCES p ON DELETE SET NULL);
CREATE UNIQUE INDEX c_u ON c (k1, k2) NULLS NOT DISTINCT;
INSERT INTO c VALUES (1, 'a', 'a'), (2, 'a', 'b');
-- @test
-- @expect-error: the second row's second nulling collides with the first row
-- @expect-message
-- @profile-lock
DELETE FROM p;
