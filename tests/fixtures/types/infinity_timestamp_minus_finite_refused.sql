-- 18.6 answers an infinite interval (`infinity`); pglet's interval has no infinity and
-- refuses (`0A000`). 18.6 accepts, so this holds only that pglet errors, whose code
-- `crates/pglet/tests/temporal_infinity.rs` pins. The `::text` keeps 18.6's answer readable to the
-- harness, whose interval decoder has no infinity.
-- @expect-error: infinite interval not modelled
-- @test
SELECT (timestamp 'infinity' - timestamp '2000-01-01')::text;
