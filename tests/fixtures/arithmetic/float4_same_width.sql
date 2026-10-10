-- `real <op> real → real`, computed in f32. pglet promoted both sides to f64 and
-- returned a `Value::Float8`, so the projection declared `real` (4 bytes) over an
-- 8-byte value — undecodable to a binary client — and printed
-- `0.1::real + 0.2::real` as `0.30000000447034836` where Postgres prints `0.3`.
-- The width and the value are one bug: Postgres's `float4_pl`/`_mi`/`_mul`/`_div`
-- (`utils/float.h`) are f32 arithmetic throughout, so rounding happens at every step.
-- Magnitudes are kept small on purpose — pglet's float text encoder never emits
-- scientific notation, a separate known divergence this fixture must not trip.
-- @setup
CREATE TABLE fw (id INT, a REAL, b REAL);
INSERT INTO fw VALUES (1, 0.1, 0.2), (2, 1.5, 2.5), (3, 3.0, 4.0);
-- @test
SELECT id, a + b AS sum, a - b AS diff, a * b AS prod, a / b AS quot
FROM fw ORDER BY id;
