-- The same build-profile divergence with the operand types swapped: an `int`
-- literal probing a `text` index coerced to `'5'` and matched, where the stripped
-- profile's `Filter(Scan)` compared `text` against `integer` and matched nothing.
-- A bare *string* literal is not affected — `resolve_expr_types` wraps it in a
-- coercing `Cast`, which is not row-independent and so is never lifted into a
-- probe. Both profiles must raise `42883`.
-- @profile-lock
-- @setup
CREATE TABLE ix_probe_txt (s TEXT);
CREATE INDEX ix_probe_txt_s ON ix_probe_txt (s);
INSERT INTO ix_probe_txt VALUES ('5'), ('6');
-- @test
-- @expect-error: no text = integer operator, on both build profiles
SELECT * FROM ix_probe_txt WHERE s = 5;
