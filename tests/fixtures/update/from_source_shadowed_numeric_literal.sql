-- The silent-**write**-loss twin of `update/from_source_shadowed_text_literal.sql`:
-- the shared column name is in the numeric family on both sides, so no operand gate
-- can fire. `ufn_src.x` is `float8` and the target's `x` is `float4`; with the source
-- unregistered, `s.x = '1.1'` cast the literal to the *target*'s `float4`, rounding it
-- to ~1.10000002384, which then promotes back to `float8` and no longer equals the
-- source value. The UPDATE matched nothing and reported success — a dropped write with
-- no error anywhere. Registering the source's relations resolves `s.x` to `float8`.
-- @setup
CREATE TABLE ufn_tgt (id INT, x REAL);
CREATE TABLE ufn_src (id INT, x FLOAT8);
INSERT INTO ufn_tgt VALUES (1, 0), (2, 0);
INSERT INTO ufn_src VALUES (1, 1.1), (2, 2.2);
-- @test
UPDATE ufn_tgt SET x = 42 FROM ufn_src s WHERE s.id = ufn_tgt.id AND s.x = '1.1' RETURNING ufn_tgt.id, ufn_tgt.x;
