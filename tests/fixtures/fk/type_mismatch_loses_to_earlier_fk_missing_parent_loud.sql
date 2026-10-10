-- An earlier key's missing parent outranks a later key's types.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42P01 relation nosuch does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (b INT REFERENCES nosuch, a TEXT REFERENCES ftp);
