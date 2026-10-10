-- A foreign key 18.6 cannot implement — referencing type `float8`, referenced key
-- `int`. `float8 → integer` is an assignment cast; `integer_ops` holds no `float8` member.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: float8 and int
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k FLOAT8 REFERENCES ftp);
