-- The largest representable numeric (`10^131071`, base-10000 weight =
-- NUMERIC_WEIGHT_MAX) parses fine — the weight cap must not over-reject an at-cap
-- literal.
-- @test
SELECT ('1e131071'::numeric > 0) AS at_cap_parses;
