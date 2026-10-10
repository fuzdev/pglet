-- The `ADD COLUMN` route runs the same gate and needs the same subordination, and its
-- checks sit in different Postgres phases: the inline `UNIQUE`'s index is named in the
-- pass phase, while the default is evaluated in the rewrite that follows it. So a taken
-- declared index name wins on both engines.
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @setup
CREATE TABLE acdn_host (a INT);
CREATE TABLE acdn_taken (q INT PRIMARY KEY);
CREATE TABLE acdn (x INT);
-- @test
ALTER TABLE acdn ADD COLUMN y INT DEFAULT nextval('acdn_host') CONSTRAINT acdn_taken_pkey UNIQUE;
-- @expect-error: the taken constraint-index name outranks the kind gate (42P07)
