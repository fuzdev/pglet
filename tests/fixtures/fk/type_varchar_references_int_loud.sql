-- A foreign key 18.6 cannot implement — referencing type `varchar`, referenced key
-- `int`. `varchar` is no `integer_ops` member and has no implicit cast to `integer`.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: varchar and int
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k VARCHAR REFERENCES ftp);
