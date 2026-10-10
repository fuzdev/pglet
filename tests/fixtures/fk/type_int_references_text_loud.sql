-- A foreign key 18.6 cannot implement — referencing type `int`, referenced key
-- `text`. `integer → text` is an assignment (I/O) conversion, not an implicit cast, and `text_ops` holds no `integer` member.
-- @setup
CREATE TABLE ftp (k TEXT PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: int and text
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k INT REFERENCES ftp);
