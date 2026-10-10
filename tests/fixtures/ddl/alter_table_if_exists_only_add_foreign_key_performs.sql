-- An inline `REFERENCES` on an added column under `ALTER TABLE IF EXISTS ONLY` is
-- enforced.
-- @setup
CREATE TABLE atie7p (id INT PRIMARY KEY);
CREATE TABLE atie7 (a INT);
ALTER TABLE IF EXISTS ONLY atie7 ADD COLUMN p INT REFERENCES atie7p (id);
-- @expect-error: the foreign key the IF EXISTS form added
-- @profile-lock
-- @test
INSERT INTO atie7 VALUES (1, 42);
