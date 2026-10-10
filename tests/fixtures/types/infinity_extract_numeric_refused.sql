-- 18.6's `EXTRACT(year …)` of an infinity is the numeric `Infinity`, which pglet's
-- numeric does not hold, so pglet refuses (`0A000`). 18.6 accepts, so this holds only
-- that pglet errors; `crates/pglet/tests/temporal_infinity.rs` pins the code. The `::text` keeps
-- 18.6's answer readable to the harness, whose numeric decoder has no `Infinity`.
-- @expect-error: numeric Infinity not modelled
-- @test
SELECT extract(year FROM date 'infinity')::text;
