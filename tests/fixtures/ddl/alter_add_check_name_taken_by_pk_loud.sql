-- `ADD CONSTRAINT … CHECK` reads the whole per-table name pool, so an existing
-- PRIMARY KEY constraint's name blocks it (42710). A gate that checked only CHECK and FK
-- names here would accept this pair.
-- @setup
CREATE TABLE aacn (a INT NOT NULL, CONSTRAINT aacn_pk PRIMARY KEY (a));

-- @expect-error: constraint aacn_pk for relation aacn already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE aacn ADD CONSTRAINT aacn_pk CHECK (a > 0);
