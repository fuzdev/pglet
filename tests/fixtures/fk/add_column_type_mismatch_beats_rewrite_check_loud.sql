-- With a rewrite too: the rewritten rows' CHECK `23514` comes after the foreign key's `42804`.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1), (2);

-- @expect-error: 42804 ahead of the rewrite's 23514
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b TEXT DEFAULT gen_random_uuid()::text CHECK (b = 'r') REFERENCES ftp;
