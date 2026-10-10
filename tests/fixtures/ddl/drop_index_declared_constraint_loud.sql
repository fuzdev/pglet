-- The refusal follows the **declared** constraint name: the index a `CONSTRAINT
-- didc_my_pk PRIMARY KEY` owns is named after the constraint, so that is the name
-- DROP INDEX refuses (2BP01).
-- @setup
CREATE TABLE didc (a INT NOT NULL, b INT);
ALTER TABLE didc ADD CONSTRAINT didc_my_pk PRIMARY KEY (a);

-- @expect-error: cannot drop index didc_my_pk because constraint didc_my_pk requires it
-- @no-error-fields: pglet's HINT names the only removal it implements; 18.6 suggests ALTER TABLE ... DROP CONSTRAINT, which pglet does not implement (it is refused 0A000)
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP INDEX didc_my_pk;
