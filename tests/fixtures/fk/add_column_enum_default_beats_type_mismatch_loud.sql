-- A constant default outside the enum is refused as the default is cooked, with the column —
-- ahead of the foreign key the column's later subcommand adds.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok');
CREATE TABLE ftp (k TEXT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1);

-- @expect-error: 22P02 invalid input value for enum mood
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b mood DEFAULT 'zz' REFERENCES ftp;
