-- `pg_attribute.atttypmod` for a sub-second precision is the bare `p` for
-- `timestamp` / `timestamptz` / `time` (`anytimestamp_typmodin` stores the precision
-- alone, with no `VARHDRSZ`) and `(INTERVAL_FULL_RANGE << 16) | p` for `interval`,
-- whose typmod packs a *range* mask beside the precision — `2147418115` for
-- `interval(3)`, `2147418116` for `interval(4)`.
--
-- An array column reports the **element's**, unadorned, on the same terms a
-- `varchar(3)[]` reports `7`; a bare `timestamp` and a `date` report `-1`.
--
-- `i`, `k`, `l` and `m` are the four `p > 6` spellings, and this is where a **clamp** is
-- separated from a **drop**: each records the clamped modifier — `6`, and `2147418118`
-- for the interval, which packs the range mask beside the clamped precision — where a
-- dropped one would read `-1`, as `e` and `f` do. The corpus's other `p > 6` positions
-- cannot make that separation, measured. At a *value* rounding at `6` is the identity, so
-- `types/precision_above_six_is_clamped.sql` says outright that it cannot; and
-- `information_schema.columns.datetime_precision` reports the **type's** `6` for a bare
-- `timestamp` as well, so `information_schema/precision_datetime_precision.sql`'s
-- `l timestamp(9)` cannot either. Both of those *do* redden on a clamp to the **wrong
-- value** — it is the **drop** neither sees.
--
-- `m`'s `2147483647` is the largest spelling the grammar's `Iconst` admits, and it alone
-- pins the clamp running *before* the narrowing to a `u8`: narrow first and the value is
-- unrepresentable and lost, which `i`, `k` and `l` cannot show — measured by mutation.
-- 18.6 emits a `WARNING` at each of the four and pglet emits none; `@setup` is not
-- compared, so that difference is outside this fixture — it is `EDGES.md` row 123's
-- residual.
-- @setup
CREATE TABLE tam (a timestamp(3), b timestamptz(2), c time(1), d interval(4),
                  e timestamp, f date, g timestamp(3)[], h interval(3)[],
                  i timestamp(9), j timestamp(6),
                  k time(9), l interval(9), m timestamp(2147483647));
-- @test
SELECT a.attname, a.atttypmod
FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
WHERE c.relname = 'tam' AND a.attnum > 0
ORDER BY a.attnum;
