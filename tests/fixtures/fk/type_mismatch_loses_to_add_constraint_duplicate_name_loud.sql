-- `ADD CONSTRAINT`'s declared name is checked first: a CHECK of that name already exists.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (b TEXT, CONSTRAINT cf CHECK (b <> ''));

-- @expect-error: 42710 constraint cf already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD CONSTRAINT cf FOREIGN KEY (b) REFERENCES ftp;
