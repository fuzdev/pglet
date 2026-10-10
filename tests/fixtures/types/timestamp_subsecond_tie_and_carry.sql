-- @test
-- The three cases that separate `rint` from truncation and from
-- half-away-from-zero: `.0000005` ties down to the even microsecond, `.0000015`
-- ties *up* to the even one, and `.9999999` rounds past the end of the second and
-- carries. The carry is why the rounding is applied to the whole instant rather
-- than to its sub-second field, where it would have nowhere to go.
-- The `::text` cast is load-bearing for the guard, not for the subject: the
-- recorded binary-leg JSON renders a plain `timestamp` at millisecond
-- precision, so the raw spelling records the two columns identically and stays
-- green on that leg with the defect present. Rendering to text pins the exact
-- microseconds on both legs.
SELECT '2026-01-01 00:00:00.0000005'::timestamp::text AS tie_down,
       '2026-01-01 00:00:00.0000015'::timestamp::text AS tie_up,
       '2026-01-01 00:00:00.9999999'::timestamp::text AS carries;
