-- `transformIndexConstraint` tests the multiplicity at the top of its own call and the
-- key repeat further down in the same call, so which answers is decided per constraint
-- in source order: constraint 1's repeat here, constraint 2's multiplicity in
-- `create_table_second_primary_key_beats_a_later_key_repeat.sql`. The same interleave
-- the key **resolution** already had with the multiplicity.
-- @expect-error: constraint 1's repeated key outranks constraint 2's multiplicity
-- @expect-message
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctkm (b INT, c INT, PRIMARY KEY (b, b), PRIMARY KEY (c));
