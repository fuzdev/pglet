-- The polarity control for `unnest_projection_non_array_beats_where_binding.sql`, one
-- scale down: **within** the expression the walk is bottom-up, so the argument's own
-- binding failure outranks the signature refusal built on it. `42883` here would mean
-- the new gate had been placed ahead of the leaves it reads — a gate that types an
-- unresolvable column and then refuses the function for having the wrong type.
--
-- Both engines answer `42703`, and did before the gate existed; what the fixture locks
-- is that adding it did not move this one.
-- @setup
CREATE TABLE unnest_na_bind (i INT);
-- @test
-- @expect-error: the argument's 42703 outranks the signature's 42883
SELECT unnest(nosuchcol) FROM unnest_na_bind;
