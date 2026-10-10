-- ...and the missing FK parent likewise: the foreign keys are added after the defaults
-- are cooked, and the cook step itself has nothing to complain about.
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE dnfk_host (a INT);
-- @test
CREATE TABLE dnfk (b INT DEFAULT nextval('dnfk_host'), z INT REFERENCES dnfk_nosuch(q));
-- @expect-error: the missing FK parent outranks the kind gate (42P01)
