-- `ADD CONSTRAINT` tests the column types before it reads a row: a row no parent holds would be
-- `23503`, and the types' `42804` comes first.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (b TEXT);
INSERT INTO ftc VALUES ('7');

-- @expect-error: 42804 before the rows' 23503
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD FOREIGN KEY (b) REFERENCES ftp (k);
