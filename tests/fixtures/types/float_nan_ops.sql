-- Postgres gives float NaN a total order for the comparison operators AND aggregates
-- (float8_cmp_internal): NaN = NaN is true, NaN sorts greater than every finite value
-- (so NaN < 5 is false, NaN > 5 is true), and MAX over a column with NaN is NaN while
-- MIN is the smallest finite value.
-- @setup
CREATE TABLE nn (id INT, f float8);
INSERT INTO nn VALUES (1, 1.0), (2, 2.0), (3, 'NaN'), (4, 3.0), (5, 'NaN');
-- @test
SELECT ('NaN'::float8 = 'NaN'::float8) AS nan_eq,   -- true
       ('NaN'::float8 > 5)             AS nan_gt,   -- true
       ('NaN'::float8 < 5)             AS nan_lt,   -- false
       max(f)                          AS max_f,    -- NaN
       min(f)                          AS min_f     -- 1
FROM nn;
