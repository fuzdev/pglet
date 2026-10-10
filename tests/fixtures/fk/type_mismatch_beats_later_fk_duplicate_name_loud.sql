-- Foreign keys are added one at a time, name then resolution then types: the first key's
-- `42804` outranks the second key's duplicate name.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 on the first key, under f
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a TEXT, b INT, CONSTRAINT f FOREIGN KEY (a) REFERENCES ftp, CONSTRAINT f FOREIGN KEY (b) REFERENCES ftp);
