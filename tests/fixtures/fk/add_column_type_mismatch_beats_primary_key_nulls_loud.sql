-- An added PRIMARY KEY column with no default is NOT NULL over NULL rows; its `23502` comes
-- after the foreign key's `42804`.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1), (2);

-- @expect-error: 42804 ahead of 23502
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b TEXT PRIMARY KEY REFERENCES ftp;
