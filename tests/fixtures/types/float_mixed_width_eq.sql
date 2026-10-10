-- Float equality spans widths with Postgres's total order: 1.5::float8 = 1.5::float4 is
-- true (a float4 promotes to f64 exactly), 0.1::float8 = 0.1::float4 is false (the two
-- widths round 0.1 differently), and 'NaN'::float8 = 'NaN'::float4 is true.
-- @test
SELECT (1.5::float8 = 1.5::float4)     AS whole_eq,   -- true
       (0.1::float8 = 0.1::float4)     AS tenth_eq,   -- false
       ('NaN'::float8 = 'NaN'::float4) AS nan_eq      -- true
;
