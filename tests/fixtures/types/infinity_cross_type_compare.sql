-- A date infinity equals the timestamp infinity of the same sign, and lies beyond the
-- other type's domain edges.
-- @test
SELECT date 'infinity' = timestamp 'infinity' AS a, date '-infinity' = timestamptz '-infinity' AS b,
       date 'infinity' > timestamp '9999-12-31 23:59:59.999999' AS c,
       timestamptz '-infinity' < date '0001-01-01' AS d,
       date 'infinity' = timestamptz '-infinity' AS e,
       timestamp 'infinity' = timestamptz 'infinity' AS f;
