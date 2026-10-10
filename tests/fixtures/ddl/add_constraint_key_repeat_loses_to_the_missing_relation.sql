-- The `ALTER TABLE … ADD CONSTRAINT` repeat outranks everything that route reaches —
-- the key columns' own `42703`, a duplicate constraint name, a taken index name, a
-- second primary key — and still loses to the relation lookup, which `AlterTable` does
-- before it transforms the subcommand at all.
-- @expect-error: the missing relation outranks the repeated key column
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
ALTER TABLE nosuchtbl_ackl ADD CONSTRAINT u UNIQUE (a, a);
