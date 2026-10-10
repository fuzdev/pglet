-- A `real`'s text is 18.6's `float4out` at the default `extra_float_digits`: the shortest
-- spelling that reads back to the same `float4` and lies strictly inside its rounding interval
-- (Ryu with `acceptBounds = false`), a tie between two shortest spellings to the even digit,
-- an integer below 2^24 spelled by its own digits, and the exponent form outside
-- `1e-4 <= |x| < 1e6`, its exponent at least two digits. Each value is one that Rust's own
-- shortest spelling answered otherwise (`155.39063`, `-84197380`, `1636696000`, `0.00001`), a
-- power of two (whose interval is lopsided), or a boundary of those rules. The `::text` is what
-- is compared: the harness reads a float column's text back as a number, so the column itself
-- would not see a spelling.
-- @setup
DROP TABLE IF EXISTS f4_text;
CREATE TABLE f4_text (k INT, v REAL);
INSERT INTO f4_text VALUES
  (1, 155.390625), (2, -84197376), (3, 1636696064), (4, 1.1), (5, 3e38), (6, 0.0001),
  (7, 1e-5), (8, 100000), (9, 1000000), (10, 16777215), (11, 16777216), (12, 1e-45),
  (13, 3.4028235e38), (14, 1.17549435e-38), (15, 123456.7), (16, 0.1), (17, -0.0),
  (21, 9.999999e-5), (22, 999999.94), (23, 1.262177448353619e-29), (24, 1.5474250491067253e26),
  (25, 89317900);
-- @test
SELECT k, v::text AS t FROM f4_text ORDER BY k;
