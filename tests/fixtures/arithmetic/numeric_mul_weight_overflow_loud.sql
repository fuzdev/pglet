-- A numeric multiply whose result exceeds Postgres's numeric format weight cap
-- (base-10000 weight past NUMERIC_WEIGHT_MAX, ~131072 integer digits) fails loud
-- (`make_result` raises "value overflows numeric format", 22003) rather than
-- building a 200001-digit result. `^` already caps; the cap now also covers `*`.
-- @test
-- @expect-error: numeric multiply weight overflow
SELECT (10::numeric ^ 100000) * (10::numeric ^ 100000);
