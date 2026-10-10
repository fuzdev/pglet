-- The `ALTER TABLE … ADD CONSTRAINT … PRIMARY KEY` spelling (the pg_dump restore
-- shape) marks its index exactly like an inline PRIMARY KEY does, so DROP INDEX is
-- refused there too.
-- @setup
CREATE TABLE diac (id INT NOT NULL, v INT);
ALTER TABLE diac ADD CONSTRAINT diac_pkey PRIMARY KEY (id);

-- @expect-error: cannot drop index diac_pkey because constraint diac_pkey requires it
-- @no-error-fields: pglet's HINT names the only removal it implements; 18.6 suggests ALTER TABLE ... DROP CONSTRAINT, which pglet does not implement (it is refused 0A000)
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP INDEX diac_pkey;
