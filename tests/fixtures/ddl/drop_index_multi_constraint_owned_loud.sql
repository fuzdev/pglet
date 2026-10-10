-- An index a declared `PRIMARY KEY` owns is refused (`2BP01`) wherever it sits in the
-- list — here second. Its reverse is `drop_index_multi_constraint_owned_reverse_loud`;
-- the pair pins that neither position lets the key be un-enforced.
-- @setup
CREATE TABLE dixc (a INT PRIMARY KEY, b INT);
CREATE INDEX dixc_b ON dixc (b);

-- @expect-error: cannot drop index dixc_pkey because constraint dixc_pkey requires it
-- @no-error-fields: pglet's HINT names the only removal it implements; 18.6 suggests ALTER TABLE ... DROP CONSTRAINT, which pglet does not implement (it is refused 0A000)
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP INDEX dixc_b, dixc_pkey;
