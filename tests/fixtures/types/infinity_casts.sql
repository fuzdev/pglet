-- An infinity casts to the same infinity between the date and timestamp types and
-- through a `(p)` modifier; it has no time of day, so `::time` is NULL.
-- @test
SELECT date 'infinity'::timestamp AS a, date '-infinity'::timestamptz AS b,
       timestamptz 'infinity'::date AS c, timestamp '-infinity'::timestamptz AS d,
       timestamp 'infinity'::timestamp(0) AS e, '-infinity'::timestamptz(3) AS f,
       (timestamp 'infinity'::time IS NULL) AS g, (timestamptz '-infinity'::time IS NULL) AS h;
