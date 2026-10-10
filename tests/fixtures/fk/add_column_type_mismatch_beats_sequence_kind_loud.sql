-- A `nextval` of a table in the added column's default is refused by 18.6 as the rewrite draws
-- it, after the foreign key's `42804`.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
INSERT INTO ftc VALUES (1);

-- @expect-error: 42804 ahead of the sequence kind refusal
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE ftc ADD COLUMN b NUMERIC DEFAULT nextval('ftp') REFERENCES ftp;
