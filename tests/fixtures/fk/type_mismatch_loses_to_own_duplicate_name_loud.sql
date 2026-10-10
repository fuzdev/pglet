-- A key's own declared name is checked before its types: the second key repeats the first's
-- name and is refused `42710` though its types are not implementable either.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42710 constraint f already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a TEXT, b INT, CONSTRAINT f FOREIGN KEY (b) REFERENCES ftp, CONSTRAINT f FOREIGN KEY (a) REFERENCES ftp);
