-- An untyped string literal in an ARRAY[...] adopts the concrete peer element's type
-- (Postgres resolves the array's element type over the typed elements, then coerces the
-- string toward it): ARRAY[1, '2'] is int[], ARRAY[true, 'false'] is bool[],
-- ARRAY[<uuid>, '<uuid-str>'] is uuid[], ARRAY[<date>, '<date-str>'] is date[]. A NULL
-- and a wider numeric peer still widen (ARRAY[1, NULL, 2.5] is numeric[]). Cast to
-- ::text so the rendered array value round-trips.
-- @test
SELECT ARRAY[1, '2']::text            AS ints,    -- {1,2}
       ARRAY[true, 'false']::text     AS bools,   -- {t,f}
       ARRAY['11111111-1111-1111-1111-111111111111'::uuid,
             '22222222-2222-2222-2222-222222222222']::text AS uuids,
       ARRAY['2026-01-01'::date, '2026-02-02']::text AS dates,  -- {2026-01-01,2026-02-02}
       ARRAY[1, NULL, 2.5]::text      AS nums     -- {1,NULL,2.5}
;
