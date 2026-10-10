-- Every pairing within the numeric family stays comparable — int2/int4/int8 and
-- float4/float8/numeric cross-compare in Postgres, so the operand gate must accept
-- all of them. Over-rejecting any pair here would be a worse bug than the silent
-- mismatch the gate closes.
-- @setup
CREATE TABLE t (a INT2, b INT4, c INT8, d FLOAT4, e FLOAT8, f NUMERIC);
INSERT INTO t VALUES (7, 7, 7, 7, 7, 7);
-- @test
SELECT a = b AS i2_i4, a = c AS i2_i8, b = c AS i4_i8,
       a = d AS i2_f4, b = e AS i4_f8, c = f AS i8_n,
       d = e AS f4_f8, d = f AS f4_n, e = f AS f8_n,
       f = b AS n_i4, a < c AS lt, e >= f AS ge
FROM t;
