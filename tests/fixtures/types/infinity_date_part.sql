-- `date_part` over an infinity: `±Infinity` for a field that grows with the value, NULL
-- for one that cycles or reads the time of day. The harness records a non-finite `float8`
-- as JSON `null`, so the binary leg is carried by the comparisons beside each value; the
-- text leg compares the values themselves.
-- @test
SELECT date_part('year', date 'infinity') AS a, date_part('epoch', date '-infinity') AS b,
       date_part('julian', timestamp 'infinity') AS c, date_part('millennium', timestamptz '-infinity') AS d,
       date_part('isoyear', date 'infinity') = 'Infinity'::float8 AS e,
       date_part('decade', timestamp '-infinity') = '-Infinity'::float8 AS f,
       date_part('century', date 'infinity') = 'Infinity'::float8 AS g,
       date_part('epoch', timestamptz 'infinity') = 'Infinity'::float8 AS g2,
       date_part('day', date 'infinity') IS NULL AS h, date_part('hour', date '-infinity') IS NULL AS i,
       date_part('timezone', timestamp 'infinity') IS NULL AS j,
       date_part('microseconds', timestamptz 'infinity') IS NULL AS k,
       date_part('dow', timestamp '-infinity') IS NULL AS l;
