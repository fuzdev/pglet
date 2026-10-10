-- `ALTER TABLE … ADD FOREIGN KEY` validating existing rows reports the child-side
-- shape, message and DETAIL alike.
-- @setup
CREATE TABLE p_fk4 (id INT PRIMARY KEY);
CREATE TABLE c_fk4 (a INT);
INSERT INTO c_fk4 VALUES (7);

-- @test
-- @expect-error: an existing row references no parent
-- @profile-lock
-- @expect-message
ALTER TABLE c_fk4 ADD CONSTRAINT c_fk4_fk FOREIGN KEY (a) REFERENCES p_fk4(id);
