-- Radix-prefixed integer literals (Postgres 16+): hex `0x`, octal `0o`, binary
-- `0b`, case-insensitive prefix, and `_` digit-group separators. Each decodes to
-- its decimal value and types by magnitude (int4 → int8 → numeric), exactly like a
-- decimal integer literal.
-- @test
SELECT
  0x1F           AS hex,
  0X1f           AS hex_upper_prefix,
  0o17           AS oct,
  0b101          AS bin,
  0x_1F          AS hex_sep_lead,
  0xFF_FF        AS hex_sep,
  0xFFFFFFFF     AS hex_bigint,
  0o777          AS oct_val,
  0b1111         AS bin_val,
  0x0            AS hex_zero
;
