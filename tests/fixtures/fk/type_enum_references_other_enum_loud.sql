-- A foreign key 18.6 cannot implement — referencing type `mood`, referenced key
-- `mood2`. Two enums, one class: `anyenum` cannot bind `mood` and `mood2` at once, though their labels
-- overlap.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok');
CREATE TYPE mood2 AS ENUM ('sad', 'ok', 'zen');
CREATE TABLE ftp (k mood2 PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: mood and mood2
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k mood REFERENCES ftp);
