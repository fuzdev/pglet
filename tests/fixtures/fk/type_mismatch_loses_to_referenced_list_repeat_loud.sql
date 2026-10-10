-- A referenced list naming one column twice is `42830`, ahead of the column types.
-- @setup
CREATE TABLE ftp (x INT, y INT, PRIMARY KEY (x, y));

-- @expect-error: 42830 referenced-columns list must not contain duplicates
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a TEXT, b INT, FOREIGN KEY (a, b) REFERENCES ftp (x, x));
