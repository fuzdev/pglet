-- A CHECK is attached as the heap is created, before any foreign key is added: its `42883`
-- outranks the foreign key written ahead of it.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42883 operator does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (b TEXT REFERENCES ftp, CHECK (b > 1));
