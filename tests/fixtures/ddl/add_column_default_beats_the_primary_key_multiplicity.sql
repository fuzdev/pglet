-- The `ADD COLUMN` cook sits ahead of the inline `PRIMARY KEY`'s multiplicity gate
-- too: over a table that already has a primary key, the default's `22P02` is what
-- 18.6 reports rather than `42P16`.
-- @no-profile-lock: the `@setup`'s index is a plain enforced PRIMARY KEY built to give
-- the `@test` a second one to collide with; the error is raised while cooking the
-- default, before index selection runs at all, so no read path differs between the
-- profiles.
-- @setup
CREATE TABLE add_pk (a INT PRIMARY KEY);
-- @expect-error: the default is cooked before the inline PRIMARY KEY is counted
-- @test
ALTER TABLE add_pk ADD COLUMN b INT PRIMARY KEY DEFAULT 'x';
