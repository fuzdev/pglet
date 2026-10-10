-- Each pair of a multi-column key is tested: the first pair (`int`/`int`) is implementable,
-- the second (`text`/`int`) is not, and the DETAIL names the second.
-- @setup
CREATE TABLE ftp (x INT, y INT, PRIMARY KEY (x, y));

-- @expect-error: 42804 names b and y
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a INT, b TEXT, FOREIGN KEY (a, b) REFERENCES ftp (x, y));
