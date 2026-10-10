-- Postgres rejects `DROP SCHEMA public` (no CASCADE) while objects still depend
-- on it (2BP01, dependent_objects_still_exist); pglet matches rather than
-- silently emptying the catalog. The reset idiom uses CASCADE for this reason.
-- @expect-error: cannot drop non-empty schema without CASCADE
-- @setup
CREATE TABLE t (id INT);
-- @test
DROP SCHEMA public;
