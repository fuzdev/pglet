-- `ALTER TABLE … ADD FOREIGN KEY` with no constraint name refuses under the name it mints.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (b TEXT);

-- @expect-error: 42804 text referencing integer
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD FOREIGN KEY (b) REFERENCES ftp (k);
