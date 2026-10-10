-- A referenced column the parent does not have is `42703`, not *there is no unique constraint
-- matching given keys*: the columns resolve before the key is looked for, on every door.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (b INT);

-- @expect-error: 42703 column nosuch referenced in foreign key constraint does not exist
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD FOREIGN KEY (b) REFERENCES ftp (nosuch);
