-- Two **different array types** in one common-type construct. Postgres resolves array
-- to array through `COERCION_PATH_ARRAYCOERCE` whenever the *element* types coerce, so
-- `int[]` and `bigint[]` unify to `bigint[]` exactly as their elements unify to
-- `bigint` — the scalar rules one dimension up, including `int[]` vs `numeric[]` →
-- `numeric[]` and `int[]` vs `real[]` → `real[]`.
-- Values are non-NULL on both sides so the binary leg asserts the elements, not a pair
-- of nulls.
-- @setup
CREATE TABLE cae (id INT, a4 INT[], a8 BIGINT[], an NUMERIC[], ar REAL[]);
INSERT INTO cae VALUES (1, ARRAY[1,2], ARRAY[3,4], ARRAY[5.5], ARRAY[6.5]);
-- @test
SELECT id,
       COALESCE(a4, a8) AS int_then_bigint,
       COALESCE(a8, a4) AS bigint_then_int,
       COALESCE(a4, an) AS int_then_numeric,
       COALESCE(a4, ar) AS int_then_real
FROM cae ORDER BY id;
