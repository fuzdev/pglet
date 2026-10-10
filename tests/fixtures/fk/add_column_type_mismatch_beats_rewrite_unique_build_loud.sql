-- With a per-row-volatile default 18.6 rewrites the table and builds the column's index after
-- the rewritten rows, so the foreign key's `42804` — tested as the key is added — outranks the
-- build's duplicate.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1), (2);

-- @expect-error: 42804 ahead of the rewrite's 23505
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b TEXT DEFAULT (length(gen_random_uuid()::text))::text UNIQUE REFERENCES ftp;
