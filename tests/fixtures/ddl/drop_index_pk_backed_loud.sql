-- DROP INDEX on the index behind a PRIMARY KEY constraint is refused (2BP01):
-- Postgres records an INTERNAL dependency from the index to its constraint, so the
-- key cannot be un-enforced by dropping its index out from under it.
-- @setup
CREATE TABLE dipk (id INT PRIMARY KEY, v INT);

-- @expect-error: cannot drop index dipk_pkey because constraint dipk_pkey requires it
-- @no-error-fields: pglet's HINT names the only removal it implements; 18.6 suggests ALTER TABLE ... DROP CONSTRAINT, which pglet does not implement (it is refused 0A000)
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP INDEX dipk_pkey;
