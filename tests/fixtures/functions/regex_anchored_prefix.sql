-- The one pattern shape the regex operators answer: `^` followed by literal characters, a
-- case-sensitive prefix test on the characters (multibyte included) — `^` alone matches
-- everything, the empty string included. Strict in both operands.
-- @test
SELECT 'abc' ~ '^ab' AS prefix, 'abc' !~ '^ab' AS negated, 'abc' ~ '^b' AS not_prefix,
  'abc' ~ '^' AS anchor_only, '' ~ '^' AS empty, 'abc' ~ '^abcd' AS longer,
  NULL ~ '^a' AS null_value, 'a' ~ NULL AS null_pattern, 'Abc' ~ '^a' AS case_sensitive,
  'é1' ~ '^é' AS multibyte;
