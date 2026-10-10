-- Two `ON UPDATE CASCADE` keys reach one row, and the image the first one writes breaks a
-- CHECK the second one's would keep.
-- @setup
CREATE TABLE p (k text PRIMARY KEY);
INSERT INTO p VALUES ('a');
CREATE TABLE c (id int, k1 text REFERENCES p ON UPDATE CASCADE, k2 text REFERENCES p ON UPDATE CASCADE, CHECK (k1 = k2));
INSERT INTO c VALUES (1, 'a', 'a');
-- @test
-- @expect-error: the first repoint breaks the CHECK
-- @expect-message
-- @profile-lock
UPDATE p SET k = 'b';
