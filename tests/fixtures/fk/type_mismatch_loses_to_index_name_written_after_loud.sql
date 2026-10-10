-- Constraint indexes are built before any foreign key is added: a taken index name is
-- `42P07` ahead of the key's `42804` though the key is written first.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE taken (q INT);

-- @expect-error: 42P07 relation taken already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a TEXT REFERENCES ftp, b INT CONSTRAINT taken UNIQUE);
