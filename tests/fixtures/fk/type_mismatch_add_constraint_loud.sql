-- `ALTER TABLE … ADD CONSTRAINT <name> FOREIGN KEY` refuses an unimplementable pair under the
-- declared name.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (b TEXT);

-- @expect-error: 42804 text referencing integer
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD CONSTRAINT ftc_link FOREIGN KEY (b) REFERENCES ftp;
