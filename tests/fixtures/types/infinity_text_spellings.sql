-- The `infinity` spellings the date and timestamp readers accept: the word in any case,
-- an optional sign with or without whitespace before the word, and whitespace around it.
-- Each column is compared in both wire formats, so the binary sentinels are checked too.
-- @test
SELECT 'infinity'::date AS d1, '-infinity'::date AS d2, '+INFINITY'::date AS d3,
       ' Infinity '::date AS d4, '- infinity'::date AS d5,
       'infinity'::timestamp AS t1, '-Infinity'::timestamp AS t2,
       'infinity'::timestamptz AS z1, E'\t-infinity\n'::timestamptz AS z2;
