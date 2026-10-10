-- Two CHECKs of one name are `42710` ahead of the foreign key written before them.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42710 constraint k already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (b TEXT REFERENCES ftp, CONSTRAINT k CHECK (b <> ''), CONSTRAINT k CHECK (b <> 'x'));
