-- ...and the duplicate inline CHECK name, named in the same phase.
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE acdc_host (a INT);
CREATE TABLE acdc (x INT);
ALTER TABLE acdc ADD CONSTRAINT acdc_n CHECK (x > 0);
-- @test
ALTER TABLE acdc ADD COLUMN y INT DEFAULT nextval('acdc_host') CONSTRAINT acdc_n CHECK (y > 0);
-- @expect-error: the duplicate CHECK name outranks the kind gate (42710)
