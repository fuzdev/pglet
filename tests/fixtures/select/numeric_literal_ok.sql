-- Regression guard for the trailing-junk check: valid numeric spellings that must
-- keep lexing cleanly — an exponent (`1e10`), a `_` digit-group separator
-- (`1_000`), a trailing dot (`5.`), a leading dot (`.5`), a signed exponent — and
-- an identifier that merely starts with a hex-looking prefix (`x1f` is a column,
-- not glued to a number).
-- @setup
CREATE TABLE t (x1f INT);
INSERT INTO t VALUES (7);

-- @test
SELECT
  1e10        AS exp,
  1_000       AS grouped,
  5.          AS trailing_dot,
  .5          AS leading_dot,
  1.5e-3      AS sci,
  9E+2        AS signed_exp,
  x1f         AS col_named_like_hex
FROM t;
