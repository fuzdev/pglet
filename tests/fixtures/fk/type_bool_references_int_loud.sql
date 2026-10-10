-- A foreign key 18.6 cannot implement — referencing type `bool`, referenced key
-- `int`. `boolean` has no equality with `integer` in any B-tree family and no implicit cast to it.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: bool and int
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k BOOL REFERENCES ftp);
