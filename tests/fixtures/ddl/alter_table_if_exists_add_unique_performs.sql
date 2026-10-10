-- `ADD CONSTRAINT … UNIQUE` under `IF EXISTS` builds the enforced index.
-- @setup
CREATE TABLE atie6 (a INT);
INSERT INTO atie6 VALUES (1);
ALTER TABLE IF EXISTS atie6 ADD CONSTRAINT atie6_a_key UNIQUE (a);
-- @expect-error: the unique key the IF EXISTS form added
-- @expect-message
-- @profile-lock
-- @test
INSERT INTO atie6 VALUES (1);
