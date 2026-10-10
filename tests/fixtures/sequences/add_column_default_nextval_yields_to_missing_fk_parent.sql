-- ...and the inline `REFERENCES` parent, resolved in that phase too.
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE acdf_host (a INT);
CREATE TABLE acdf (x INT);
-- @test
ALTER TABLE acdf ADD COLUMN y INT DEFAULT nextval('acdf_host') REFERENCES acdf_nosuch(q);
-- @expect-error: the missing FK parent outranks the kind gate (42P01)
