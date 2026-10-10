-- Without a rewrite the CHECK is judged in the verification scan after the foreign key is added:
-- `42804` ahead of the backfill's `23514`.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1), (2);

-- @expect-error: 42804 ahead of 23514
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b TEXT DEFAULT 'q' CHECK (b = 'r') REFERENCES ftp;
