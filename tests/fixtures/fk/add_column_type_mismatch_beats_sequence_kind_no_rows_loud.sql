-- The same over a table with no rows, where 18.6 never draws the default and pglet's own kind
-- gate stands in for the first write's refusal: the foreign key's `42804` still comes first.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);

-- @expect-error: 42804 ahead of the sequence kind refusal
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b NUMERIC DEFAULT nextval('ftp') REFERENCES ftp;
