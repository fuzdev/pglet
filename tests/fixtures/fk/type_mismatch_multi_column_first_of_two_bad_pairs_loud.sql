-- Two unimplementable pairs: the first in key order is the one named.
-- @setup
CREATE TABLE ftp (x INT, y INT, PRIMARY KEY (x, y));

-- @expect-error: 42804 names a and x
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (a NUMERIC, b TEXT, FOREIGN KEY (a, b) REFERENCES ftp (x, y));
