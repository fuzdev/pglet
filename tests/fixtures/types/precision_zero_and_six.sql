-- `p = 0` rounds to whole seconds (carrying, so `…05.6789` is `…06` and `…05.4999999`
-- is `…06` too — the µs value is `5.5` after the input function's own rounding).
-- `p = 6` is the **control**: the maximum precision, where the value is untouched on
-- both engines. Only one of 18.6's three functions gets there by returning early —
-- `AdjustTimestampForTypmod` skips the arm at `MAX_TIMESTAMP_PRECISION`, while
-- `AdjustTimeForTypmod` and `AdjustIntervalForTypmod` run it with offset `0` and scale
-- `1`, which is the identity. It is a *captured* modifier all the same —
-- see `catalog/precision_atttypmod.sql` for the `atttypmod = 6` that separates it from a
-- bare `timestamp`'s `-1`. `information_schema.columns.datetime_precision` cannot make
-- that separation — it reports the *type*, so it answers `6` for both.
-- @test
SELECT '2020-01-02 03:04:05.6789'::timestamp(0) AS a,
       '2020-01-02 03:04:05.4999999'::timestamp(0) AS b,
       '2020-01-02 03:04:05.5'::timestamp(0) AS c,
       '2020-01-02 03:04:05.6789'::timestamp(6) AS d,
       '03:04:05.6789'::time(6) AS e,
       '1.23456 secs'::interval(6) AS f;
