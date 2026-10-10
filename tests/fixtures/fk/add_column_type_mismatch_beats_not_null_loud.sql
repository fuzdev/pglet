-- A NOT NULL column with no default over rows is `23502` in the verification scan, after the
-- foreign key's `42804` — the index the UNIQUE builds over the NULLs fails nothing first.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1), (2);

-- @expect-error: 42804 ahead of 23502
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b TEXT NOT NULL UNIQUE REFERENCES ftp;
