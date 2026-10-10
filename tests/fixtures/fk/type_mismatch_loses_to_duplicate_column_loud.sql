-- A repeated column name is parse analysis's `42701`, ahead of every constraint.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42701 column b specified more than once
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (b TEXT REFERENCES ftp, b INT);
