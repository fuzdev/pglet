-- A `timestamp(p)` / `timestamptz(p)` cast **rounds** the value's microseconds to `p`
-- fractional digits rather than discarding the modifier. All four spellings of the
-- target reach the same node: the short one, the two-word `without`/`with time zone`
-- forms, and `CAST(… AS …)`.
-- @test
SELECT '2020-01-02 03:04:05.6789'::timestamp(3) AS a,
       '2020-01-02 03:04:05.6789'::timestamptz(3) AS b,
       '2020-01-02 03:04:05.6789'::timestamp(3) without time zone AS c,
       '2020-01-02 03:04:05.6789'::timestamp(3) with time zone AS d,
       CAST('2020-01-02 03:04:05.6789' AS timestamp(3)) AS e;
