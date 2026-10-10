-- `ADD FOREIGN KEY` validates the rows already there by the same equality.
-- @profile-lock
-- @expect-error: the float8 1.1 has no real parent (23503)
-- @expect-message
-- @setup
CREATE TABLE fkpd (x REAL PRIMARY KEY);
INSERT INTO fkpd VALUES (1.1);
CREATE TABLE fkcd (x FLOAT8);
INSERT INTO fkcd VALUES (1.1);
-- @test
ALTER TABLE fkcd ADD CONSTRAINT fkcd_fk FOREIGN KEY (x) REFERENCES fkpd (x);
