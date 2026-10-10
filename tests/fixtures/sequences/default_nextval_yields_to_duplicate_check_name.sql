-- The DDL-time kind gate on a user's `DEFAULT nextval('<a table>')` is a deliberate
-- over-rejection standing in for the 42809 Postgres raises at the first *write*, so it
-- must not outrank an error Postgres raises at DDL. Here the duplicate CHECK name wins
-- on both engines: Postgres cooks the default (which merely resolves the regclass and
-- succeeds), then names the constraints. pglet used to answer 42809.
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE dnck_host (a INT);
-- @test
CREATE TABLE dnck (b INT DEFAULT nextval('dnck_host'), CONSTRAINT dnck_n CHECK (b > 0), CONSTRAINT dnck_n CHECK (b > 1));
-- @expect-error: the duplicate CHECK name outranks the kind gate (42710)
