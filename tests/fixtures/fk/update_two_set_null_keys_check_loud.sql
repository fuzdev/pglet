-- Two `ON UPDATE SET NULL` keys null one row twice, and the second image, nulled over the
-- first, breaks a CHECK the first one keeps.
-- @setup
CREATE TABLE p (k text PRIMARY KEY);
INSERT INTO p VALUES ('a');
CREATE TABLE c (id int, k1 text REFERENCES p ON UPDATE SET NULL, k2 text REFERENCES p ON UPDATE SET NULL, CHECK (k1 IS NOT NULL OR k2 IS NOT NULL));
INSERT INTO c VALUES (1, 'a', 'a');
-- @test
-- @expect-error: the row nulled by both keys breaks its CHECK
-- @expect-message
-- @profile-lock
UPDATE p SET k = 'b';
