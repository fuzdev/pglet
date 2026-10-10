-- A foreign key 18.6 cannot implement — referencing type `text`, referenced key
-- `jsonb`. `jsonb_ops` holds only `jsonb`, and `text` has no implicit cast to it.
-- @setup
CREATE TABLE ftp (k JSONB PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: text and jsonb
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k TEXT REFERENCES ftp);
