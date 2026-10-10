-- A foreign key 18.6 cannot implement — referencing type `numeric`, referenced key
-- `int`. `numeric → integer` is an assignment cast, not an implicit one, so the narrowing direction is refused.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: numeric and int
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k NUMERIC REFERENCES ftp);
