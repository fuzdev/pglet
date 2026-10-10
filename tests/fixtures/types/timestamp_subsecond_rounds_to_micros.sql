-- @test
-- Postgres's `Timestamp` is an i64 of microseconds, so `ParseFraction` rounds a
-- sub-microsecond literal with `rint` — ties to *even*, not away from zero. pglet
-- used to keep the nanoseconds and truncate at the encoders, so `b` rendered
-- `.123456` here against 18.6's `.123457`: a silently wrong value on a literal
-- both engines accept.
-- The `::text` cast is load-bearing for the guard, not for the subject: the
-- recorded binary-leg JSON renders a plain `timestamp` at millisecond
-- precision, so the raw spelling records the two columns identically and stays
-- green on that leg with the defect present. Rendering to text pins the exact
-- microseconds on both legs.
SELECT '2026-01-01 00:00:00.1234564'::timestamp::text AS a,
       '2026-01-01 00:00:00.1234566'::timestamp::text AS b;
