-- The first key's `42804` outranks a later key's missing parent.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 on the first key
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a TEXT REFERENCES ftp, b INT REFERENCES nosuch);
