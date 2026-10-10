-- A foreign key 18.6 cannot implement — referencing type `mood`, referenced key
-- `text`. An enum is read by its identity, not its storage: `mood` has no implicit cast to `text`, and
-- `text_ops` holds no `mood` member.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok');
CREATE TABLE ftp (k TEXT PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: mood and text
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k mood REFERENCES ftp);
