-- A string that starts `\x` is the hex format: digit pairs in either case, with a space, a
-- tab, a newline or a carriage return allowed where a pair would start. The whitespace
-- columns are `E''` strings so the literal can hold a tab and a newline; `E'\\x'` is the
-- two characters `\x`.
-- @test
SELECT
  '\x'::bytea AS no_digits,
  '\x6162'::bytea AS pairs,
  '\xDeadBEEF'::bytea AS mixed_case,
  '\x6162 63'::bytea AS space_between_pairs,
  '\x 61  62 '::bytea AS spaces_around_pairs,
  '\x   '::bytea AS whitespace_only,
  E'\\x61\t62\n63\r64'::bytea AS tab_newline_return,
  '\x5c78'::bytea AS the_prefix_as_bytes;
