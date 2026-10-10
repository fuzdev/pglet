-- A `double precision`'s text is 18.6's `float8out` at the default `extra_float_digits`: the
-- shortest spelling that reads back to the same `float8` and lies strictly inside its rounding
-- interval, an integer below 2^53 spelled by its own digits, and the exponent form outside
-- `1e-4 <= |x| < 1e15`, its exponent at least two digits. Rust's own shortest spelling answered
-- `100000000000000000000` for `1e20` and `0.0000000001` for `1e-10`; the rest are boundaries of
-- those rules, powers of two (`2^-24` is a tie whose even spelling falls outside its lopsided
-- interval), and a value whose Rust spelling sits on the interval's boundary. The `::text` is
-- what is compared: the harness reads a float column's text back as a number.
-- @setup
DROP TABLE IF EXISTS f8_text;
CREATE TABLE f8_text (k INT, v DOUBLE PRECISION);
INSERT INTO f8_text VALUES
  (1, 1e20), (2, 1e-10), (3, 0.1), (4, 1e15), (5, 1e16), (6, 123456789012345),
  (7, 9007199254740991), (8, 9007199254740993), (9, 5e-324), (10, 1.7976931348623157e308),
  (11, 2.2250738585072014e-308), (12, 0.0001), (13, 9.9999999999999e-5), (14, -0.0),
  (17, 1e100), (18, 12345678.9), (19, 1.5e300), (20, 1e-7),
  (21, 5.960464477539063e-8), (22, -2.596472244501881e16), (23, 7.120236347223045e-307);
-- @test
SELECT k, v::text AS t FROM f8_text ORDER BY k;
