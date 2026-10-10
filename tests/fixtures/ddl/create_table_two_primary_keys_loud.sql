-- A table may have exactly one PRIMARY KEY. Postgres says so with its own SQLSTATE
-- (42P16, `multiple primary keys for table "<t>" are not allowed`) rather than a name
-- collision — the two keys can be perfectly well named, and only the *non-PK*
-- redundancy is merged.
-- @expect-error: multiple primary keys for table cttp are not allowed
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE cttp (a INT NOT NULL PRIMARY KEY, b INT NOT NULL PRIMARY KEY);
