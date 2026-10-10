-- A composite `MATCH SIMPLE` child holding a NULL references nothing, so a cascade can
-- repoint another child onto its non-NULL part, which a unique index then holds twice.
-- @setup
CREATE TABLE p (a text, b text, PRIMARY KEY (a, b));
CREATE TABLE c (id int, a text, b text, FOREIGN KEY (a, b) REFERENCES p ON UPDATE CASCADE);
CREATE UNIQUE INDEX c_u ON c (a);
INSERT INTO p VALUES ('x', 'y');
INSERT INTO c VALUES (1, 'x', 'y'), (2, 'z', NULL);
-- @test
-- @expect-error: the repointed a duplicates the NULL-holding row
-- @expect-message
-- @profile-lock
UPDATE p SET a = 'z';
