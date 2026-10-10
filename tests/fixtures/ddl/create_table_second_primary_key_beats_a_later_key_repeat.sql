-- The mirror arrangement: constraint 2's multiplicity is tested before its own keys are
-- walked, so `42P16` wins. Read against
-- `create_table_key_repeat_beats_a_second_primary_key.sql` — the pair pins the
-- per-constraint interleave, and neither arrangement follows from the other.
-- @expect-error: constraint 2's multiplicity outranks its own repeated key
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctsm (b INT, c INT, PRIMARY KEY (c), PRIMARY KEY (b, b));
