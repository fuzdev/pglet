-- An added column's constraint index name is claimed before the foreign key is added: a taken
-- name is `42P07` ahead of the key's `42804`.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
CREATE TABLE taken (q INT);

-- @expect-error: 42P07 relation taken already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b TEXT CONSTRAINT taken UNIQUE REFERENCES ftp;
