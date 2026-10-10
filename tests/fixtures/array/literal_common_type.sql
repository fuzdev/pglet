-- An ARRAY[...] literal's element type is the common supertype of all elements
-- (widest numeric, text-family unification), not the first element's: ARRAY[1, 2.5]
-- widens the whole array to numeric[] (value {1,2.5}, not the truncated {1,3} a
-- first-element int4 would give) and ARRAY[1::int8, 2::int4] is bigint[]. Cast to
-- ::text so the array's rendered value round-trips regardless of wire-decode support.
-- @test
SELECT ARRAY[1, 2.5, 3]::text        AS mixed_numeric,   -- {1,2.5,3}
       ARRAY[1::int8, 2::int4]::text AS mixed_int,       -- {1,2}
       ARRAY[1, 2.5]::text           AS pair             -- {1,2.5}
;
