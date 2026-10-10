-- A referencing column the table does not have is `42703`, ahead of the column types.
-- @setup
CREATE TABLE ftp (x INT, y INT, PRIMARY KEY (x, y));

-- @expect-error: 42703 column nosuch referenced in foreign key constraint does not exist
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a TEXT, b INT, FOREIGN KEY (a, nosuch) REFERENCES ftp (x, y));
