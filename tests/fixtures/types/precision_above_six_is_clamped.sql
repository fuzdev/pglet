-- A precision above the type's maximum is **accepted**, not refused: 18.6 warns
-- (`precision reduced to maximum allowed, 6` — pglet has no NOTICE channel to reproduce
-- it) and clamps it to `6`, at a cast as at a column — both measured. `2147483647` is the
-- largest spelling its grammar's `Iconst` admits, and clamps the same way.
--
-- **These four columns pin acceptance and the value, and not the clamp.** Rounding at
-- `6` is the identity here, and so is rounding at `5`: the widest fraction below is
-- `1.23456 secs`, so any recorded precision of 5 or more — and equally the *absence* of
-- one — produces exactly these four values. Measured by mutation: a build that discards a
-- temporal `(p)` outright reddens most of the `precision_*` corpus and leaves this fixture
-- green, and so does one that clamps at `5`. What separates a **clamp** from a **drop**
-- is therefore `pg_attribute.atttypmod` and not the value, and
-- `catalog/precision_atttypmod.sql` is where all four of these spellings are pinned to
-- the clamped modifier they record — `6`, and `interval`'s `2147418118` — against the
-- `-1` of a column carrying no modifier at all.
-- @test
SELECT '2020-01-02 03:04:05.6789'::timestamp(9) AS a,
       '2020-01-02 03:04:05.6789'::timestamp(2147483647) AS b,
       '03:04:05.6789'::time(9) AS c,
       '1.23456 secs'::interval(9) AS d;
