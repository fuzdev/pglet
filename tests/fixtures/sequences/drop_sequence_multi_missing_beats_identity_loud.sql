-- Phase 1 outranks phase 2 for sequences too: a missing name written after the
-- identity sequence answers `42P01`, not the `2BP01` that sequence earns on its own
-- (`drop_sequence_multi_identity_loud.sql` is that polarity). Red on doing dependency
-- resolution inside the phase-1 loop.
-- @setup
CREATE TABLE dsmi (id INT GENERATED ALWAYS AS IDENTITY, b INT);

-- @expect-error: sequence "dsmi_nosuch" does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP SEQUENCE dsmi_id_seq, dsmi_nosuch;
