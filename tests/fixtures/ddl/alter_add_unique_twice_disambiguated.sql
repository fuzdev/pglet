-- The merge is scoped to one statement's own constraint list, which is Postgres's
-- scope too ("XXX in ALTER TABLE case, it'd be nice to look for duplicate
-- pre-existing indexes, too"). Two separate `ADD UNIQUE (a)` statements therefore
-- build two indexes, the second under a disambiguated auto-name.
-- @setup
CREATE TABLE aaut (a INT);
ALTER TABLE aaut ADD UNIQUE (a);
ALTER TABLE aaut ADD UNIQUE (a);

-- @test
SELECT indexname::text AS indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'aaut' ORDER BY indexname;
