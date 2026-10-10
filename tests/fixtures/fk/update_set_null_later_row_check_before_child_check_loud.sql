-- An `ON UPDATE SET NULL` fires once the statement's rows are written, so a later target
-- row's CHECK outranks the nulled child's own CHECK.
-- @setup
CREATE TABLE p (k text PRIMARY KEY, v int CHECK (v > 0));
CREATE TABLE c (k text CHECK (k IS NOT NULL) REFERENCES p ON UPDATE SET NULL);
INSERT INTO p VALUES ('a', 1), ('b', 1);
INSERT INTO c VALUES ('a');
-- @test
-- @expect-error: the parent's CHECK, not the child's
-- @expect-message
-- @profile-lock
UPDATE p SET k = CASE WHEN k = 'a' THEN 'az' ELSE k END, v = CASE WHEN k = 'b' THEN -1 ELSE v END;
