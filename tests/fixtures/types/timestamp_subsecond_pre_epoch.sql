-- @test
-- Round-half-to-even is symmetric about zero, so a pre-epoch instant needs no
-- separate arm — and the tie carries across the epoch boundary. The two spellings
-- differ by one nanosecond and land two microseconds apart, which is what pins the
-- tie direction rather than merely the rounding.
-- The `::text` cast is load-bearing for the guard, not for the subject: the
-- recorded binary-leg JSON renders a plain `timestamp` at millisecond
-- precision, so the raw spelling records the two columns identically and stays
-- green on that leg with the defect present. Rendering to text pins the exact
-- microseconds on both legs.
SELECT '1969-12-31 23:59:59.9999995'::timestamp::text AS tie_carries_to_epoch,
       '1969-12-31 23:59:59.9999985'::timestamp::text AS tie_down;
