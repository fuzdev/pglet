-- The `timetz(3)` spelling of the same refusal — a different word reaching the same
-- canonical `time with time zone(3)`, and the same `42704`. 18.6 accepts it, so any
-- pglet error passes; the `::text` wrapper is the harness's (see
-- `precision_timetz_still_unsupported_loud.sql`).
-- @expect-error: timetz(3) stays unsupported
-- @test
SELECT ('03:04:05'::timetz(3))::text;
