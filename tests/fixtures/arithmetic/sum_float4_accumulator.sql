-- `sum(real)` accumulates in **f32**: `pg_aggregate.dat` gives it
-- `aggtransfn => float4pl, aggtranstype => float4`, so every partial sum is rounded to
-- f32. That changes the answer, not just the width — pglet accumulated in f64 and
-- returned e.g. `1.0000000149011612` where Postgres returns `1` for ten `0.1::real`s,
-- and `12.999999761581421` where Postgres returns `13`. The width was wrong too: a
-- `Value::Float8` under the `real` type `aggregate_result_type` declares, which a
-- binary client cannot decode.
-- `avg`/`min`/`max`/`count` are the controls. Postgres's `float4_accum` transitions
-- through float8, so `avg(real)` really is `double precision` and pglet already matched
-- it — this must not change with `sum`.
-- Magnitudes stay small deliberately: pglet's float text encoder never emits scientific
-- notation (a separate known divergence), which `2^24`-scale data would trip.
-- @setup
CREATE TABLE fs (g INT, v REAL);
INSERT INTO fs VALUES (1,0.1),(1,0.1),(1,0.1),(1,0.1),(1,0.1),
                      (1,0.1),(1,0.1),(1,0.1),(1,0.1),(1,0.1),
                      (2,1.3),(2,2.6),(2,3.9),(2,5.2);
-- @test
SELECT g, sum(v) AS s, avg(v) AS a, min(v) AS mn, max(v) AS mx, count(v) AS c
FROM fs GROUP BY g ORDER BY g;
