-- A foreign key 18.6 cannot implement — referencing type `text`, referenced key
-- `int`. No `integer_ops` member is `text`, and `text` has no implicit cast to `integer`.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: text and int
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k TEXT REFERENCES ftp);
