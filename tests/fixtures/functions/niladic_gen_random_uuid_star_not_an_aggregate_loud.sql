-- The UUID spelling of `niladic_now_star_not_an_aggregate_loud.sql`, and the control
-- that identifies the rule as "the name carries a zero-argument signature" rather than
-- "the name is in the timestamp family". `upper(*)` and `sum(*)` are 42883 on 18.6 by
-- the same rule, because neither `upper()` nor `sum()` exists — pglet answers 0A000 to
-- those, a pre-existing loud divergence recorded in EDGES.md row 45 and deliberately untouched.
-- @test
-- @expect-error: gen_random_uuid(*) specified, but gen_random_uuid is not an aggregate function
SELECT gen_random_uuid(*);
