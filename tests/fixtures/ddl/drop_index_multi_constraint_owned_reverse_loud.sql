-- The reverse of `drop_index_multi_constraint_owned_loud.sql` — the constraint's index
-- written first.
-- @setup
CREATE TABLE dixr (a INT PRIMARY KEY, b INT);
CREATE INDEX dixr_b ON dixr (b);

-- @expect-error: cannot drop index dixr_pkey because constraint dixr_pkey requires it
-- @no-error-fields: pglet's HINT names the only removal it implements; 18.6 suggests ALTER TABLE ... DROP CONSTRAINT, which pglet does not implement (it is refused 0A000)
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP INDEX dixr_pkey, dixr_b;
