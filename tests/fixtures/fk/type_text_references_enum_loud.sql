-- A foreign key 18.6 cannot implement — referencing type `text`, referenced key
-- `mood`. `enum_ops`'s input type is `anyenum`, which `text` cannot bind.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok');
CREATE TABLE ftp (k mood PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: text and mood
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k TEXT REFERENCES ftp);
