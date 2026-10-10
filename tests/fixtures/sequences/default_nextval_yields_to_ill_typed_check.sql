-- ...and the CHECK expression's own type error, which is raised by the cook-then-name
-- loop rather than by either name gate.
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE dnit_host (a INT);
-- @test
CREATE TABLE dnit (b INT DEFAULT nextval('dnit_host'), CHECK (b LIKE 'x%'));
-- @expect-error: the ill-typed CHECK outranks the kind gate (42883)
