-- No unique key covering the referenced column is `42830`, ahead of the column types.
-- @setup
CREATE TABLE ftp (k INT);

-- @expect-error: 42830 no unique constraint matching given keys
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (b TEXT REFERENCES ftp (k));
