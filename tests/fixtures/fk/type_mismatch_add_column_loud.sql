-- `ALTER TABLE … ADD COLUMN … REFERENCES` refuses an unimplementable pair: the added column is
-- the referencing one, typed before it reaches the catalog.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);

-- @expect-error: 42804 text referencing integer
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b TEXT REFERENCES ftp;
