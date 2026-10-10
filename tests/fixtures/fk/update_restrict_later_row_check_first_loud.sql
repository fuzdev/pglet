-- An `ON UPDATE RESTRICT` key is checked once the statement's rows are written, so a
-- later target row's CHECK outranks the earlier row's `23001`.
-- @setup
CREATE TABLE p (k text PRIMARY KEY, v int CHECK (v > 0));
CREATE TABLE c (k text REFERENCES p ON UPDATE RESTRICT);
INSERT INTO p VALUES ('a', 1), ('b', 1);
INSERT INTO c VALUES ('a');
-- @test
-- @expect-error: the second row's CHECK outranks the RESTRICT key
-- @expect-message
-- @profile-lock
UPDATE p SET k = CASE WHEN k = 'a' THEN 'az' ELSE k END, v = CASE WHEN k = 'b' THEN -1 ELSE v END;
