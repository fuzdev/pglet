-- A SERIAL column referencing a `text` key: the rewrite draws the sequence, and the foreign key's
-- `42804` comes first.
-- @setup
CREATE TABLE ftp (k TEXT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1);

-- @expect-error: 42804 integer referencing text
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b SERIAL REFERENCES ftp;
