-- `NULLIF` does **not** use the common-type rule its IR neighbours `greatest`/`least`
-- use. Postgres resolves the `=` operator and then declares *argument one's* type as
-- the resolution left it (`parse_expr.c`: `opresulttype = exprType(linitial(args))`).
-- So `NULLIF(int4, int8)` is `integer` where `GREATEST(int4, int8)` is `bigint` —
-- pglet unified both arguments and declared the widened type.
-- Argument one survives whenever a registered `=` takes it on the left (the integer
-- family cross-compares, and a float accepts any numeric); it widens when none does —
-- `NULLIF(int4, real)` is `double precision` because there is no `int4 = float4`.
-- Two cases that look like they should survive and do not: **`varchar` has no `=`
-- operator of its own** (comparison goes through `text`), so argument one is coerced
-- even against another `varchar` and the result is `text`; and `time` against
-- `interval` resolves to `interval`, the one implicit cast crossing a type-category
-- line, which the *operator* rulebook bridges and the common-type rule does not.
-- @setup
CREATE TABLE nr (id INT, i2 SMALLINT, i4 INT, i8 BIGINT, r REAL, n NUMERIC,
                 v VARCHAR(10), v2 VARCHAR(10), t TEXT, tm TIME, iv INTERVAL);
INSERT INTO nr VALUES (1, 1, 10, 20, 1.5, 2.5, 'a', 'b', 'c', '10:00:00', '1 day');
-- @test
SELECT id,
       NULLIF(i4, i8) AS i4_i8,
       NULLIF(i8, i4) AS i8_i4,
       NULLIF(i2, i8) AS i2_i8,
       NULLIF(r, i4) AS real_int,
       NULLIF(i4, r) AS int_real,
       NULLIF(n, i4) AS numeric_int,
       NULLIF(i4, n) AS int_numeric,
       NULLIF(v, v2) AS varchar_varchar,
       NULLIF(v, t) AS varchar_text,
       NULLIF(t, v) AS text_varchar,
       NULLIF(tm, iv) AS time_interval,
       NULLIF(iv, tm) AS interval_time,
       GREATEST(i4, i8) AS greatest_i4_i8
FROM nr ORDER BY id;
