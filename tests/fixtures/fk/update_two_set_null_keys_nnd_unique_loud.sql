-- Two `ON UPDATE SET NULL` keys null one row twice, the second over the first, and each
-- nulled row is entered into the unique index before the next: the first row ends
-- `(NULL, NULL)`, and the second row's second nulling collides with it under
-- `NULLS NOT DISTINCT`.
-- @setup
CREATE TABLE p (k text PRIMARY KEY);
INSERT INTO p VALUES ('a'), ('b');
CREATE TABLE c (id int, k1 text REFERENCES p ON UPDATE SET NULL, k2 text REFERENCES p ON UPDATE SET NULL);
CREATE UNIQUE INDEX c_u ON c (k1, k2) NULLS NOT DISTINCT;
INSERT INTO c VALUES (1, 'a', 'a'), (2, 'a', 'b');
-- @test
-- @expect-error: the second row's second nulling collides with the first row
-- @expect-message
-- @profile-lock
UPDATE p SET k = k || 'z';
