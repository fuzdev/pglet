-- The message names the constraint under the name it was minted, not its base: a CHECK has
-- already taken `ftc_b_fkey`, so the foreign key is `ftc_b_fkey1`.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 under ftc_b_fkey1
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (b TEXT REFERENCES ftp, CONSTRAINT ftc_b_fkey CHECK (b <> ''));
