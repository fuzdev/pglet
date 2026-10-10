-- The text of an infinity is the word at the renderers here: the `::text` cast, a
-- `timestamptz` (no `+00` offset), and an array element.
-- @text
-- @test
SELECT 'infinity'::date::text AS d, '-infinity'::timestamp::varchar AS t,
       'infinity'::timestamptz AS z, '-infinity'::timestamptz::text AS zt,
       '{infinity,-infinity,2000-01-01}'::date[] AS da,
       ARRAY['-infinity'::timestamptz, 'infinity'] AS za;
