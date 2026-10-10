-- The `UNIQUE` arm of the `ALTER TABLE … ADD CONSTRAINT` route, with 18.6's own
-- *unique constraint* wording.
-- @setup
CREATE TABLE acur (a INT);
-- @expect-error: a repeated column in a unique constraint
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE acur ADD CONSTRAINT acur_u UNIQUE (a, a);
