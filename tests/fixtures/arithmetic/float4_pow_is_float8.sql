-- `^` has only two operators in Postgres — `^(float8, float8)` and
-- `^(numeric, numeric)` — so `real ^ real` is **double precision**, not `real`.
-- pglet inferred the arithmetic promotion (`real`) while `eval_power` returned a
-- `Value::Float8`: 8 bytes under a 4-byte declaration. `int ^ int` is also float8
-- (there is no integer exponentiation), and only numeric-vs-numeric stays numeric.
-- @setup
CREATE TABLE fp (id INT, a REAL, b REAL, n NUMERIC);
INSERT INTO fp VALUES (1, 2.0, 3.0, 2.0);
-- @test
SELECT id, a ^ b AS real_pow, 2 ^ 3 AS int_pow, n ^ 2::NUMERIC AS numeric_pow
FROM fp ORDER BY id;
