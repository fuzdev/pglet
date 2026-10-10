-- Phase 1 reports the POSITIONALLY FIRST failure: a missing name written before a
-- wrong-kind one is `42P01`. Mirrored by
-- `drop_table_multi_positional_wrong_kind_first_loud.sql` (`42809`) over the same
-- setup — neither fixture discriminates alone, since `@expect-error` compares only the
-- SQLSTATE and an engine that always reported one of the two would pass either.
-- @setup
CREATE SEQUENCE dpm_seq;

-- @expect-error: table "dpm_nosuch" does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE dpm_nosuch, dpm_seq;
