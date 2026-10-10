-- An omitted referenced list resolves to the parent's two-column primary key, and the arity's
-- `42830` outranks the first pair's types.
-- @setup
CREATE TABLE ftp (x INT, y INT, PRIMARY KEY (x, y));

-- @expect-error: 42830 number of referencing and referenced columns disagree
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a TEXT REFERENCES ftp);
