-- One-argument ceil/floor/round/trunc on an INTEGER resolve to Postgres's double
-- precision overload (ceil(5) → 5.0), NOT numeric. The two-argument round(x,s) /
-- trunc(x,s) have only a (numeric, integer) overload, so they yield numeric (rendered
-- with the scale, 5.00). A numeric-literal argument stays numeric (ceil(5.5) → 6).
-- The numeric-vs-float8 result type is guarded byte-for-byte: a float8 renders "5",
-- a numeric renders "5.00".
-- @test
SELECT ceil(5)     AS ce,   -- 5   (float8)
       floor(7)    AS fl,   -- 7   (float8)
       round(9)    AS r0,   -- 9   (float8)
       trunc(4)    AS tr,   -- 4   (float8)
       round(5, 2) AS r2,   -- 5.00 (numeric)
       trunc(5, 2) AS t2,   -- 5.00 (numeric)
       ceil(5.5)   AS cn    -- 6   (numeric)
;
