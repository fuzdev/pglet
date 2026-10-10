-- The accept side of the same set, over every member of `boolin`'s C `isspace` trim
-- set. The vertical tab is spelled `\x0B` here because this fixture's subject is the
-- trim set and not the escape that reaches it: `\v`, `\x0B`, `\013` and `\u000B` each
-- decode to U+000B on both engines, and the escape has its own guard in
-- `string_escape/vertical_tab_escape.sql`. `\b` decodes and is deliberately absent: it is not
-- C `isspace`, and `E'\bt\b'::bool` is `22P02` on both engines rather than a trim.
-- @test
SELECT ' t '::bool AS space, E'\tt\t'::bool AS tab, E'\nt\n'::bool AS newline,
       E'\rt\r'::bool AS carriage_return, E'\ft\f'::bool AS form_feed,
       E'\x0Bt\x0B'::bool AS vertical_tab;
