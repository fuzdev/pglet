-- ...and the same namespace read from the other side: a CREATE TABLE whose name an
-- index already holds is 42P07 (`heap_create_with_catalog`).
-- @setup
CREATE TABLE ctni (a INT);
CREATE INDEX ctni_idx ON ctni (a);

-- @expect-error: relation ctni_idx already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctni_idx (z INT);
