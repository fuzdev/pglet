-- 18.6's `quote_identifier` rule on the inputs beside the keyword list: bare only for
-- `a`–`z` / `_` first and `a`–`z` / `0`–`9` / `_` throughout (ASCII only, so a non-ASCII
-- letter is quoted), every `"` doubled inside the quotes, the empty name `""`. Strict.
-- @test
SELECT
  quote_ident('abc') AS plain, quote_ident('Abc') AS upper, quote_ident('a b') AS space,
  quote_ident('a"b') AS quote, quote_ident('') AS empty, quote_ident('1a') AS digit_first,
  quote_ident('_a') AS underscore, quote_ident('a1_') AS digits, quote_ident('é') AS non_ascii,
  quote_ident('aé') AS non_ascii_tail, quote_ident('a$') AS dollar, quote_ident(NULL) AS null_arg,
  quote_ident('action') AS unreserved, quote_ident('left') AS type_func, quote_ident('int') AS col_name;
