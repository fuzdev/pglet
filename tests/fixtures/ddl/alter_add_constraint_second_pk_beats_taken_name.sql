-- On the ALTER path the order is Postgres's own: `index_check_primary_key` runs
-- inside `DefineIndex` *before* `index_create` looks at the name, so a statement that
-- is both a second primary key and a taken relation name reports 42P16, not 42P07.
-- @setup
CREATE TABLE aacb (a INT NOT NULL, b INT NOT NULL, PRIMARY KEY (a));
CREATE INDEX aacb_taken ON aacb (b);

-- @expect-error: multiple primary keys for table aacb are not allowed
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE aacb ADD CONSTRAINT aacb_taken PRIMARY KEY (b);
