-- The `SET` classifier runs ahead of the parser (`runs_outside_the_planner`), so its
-- case fold is the lexer's ASCII width. U+017F LATIN SMALL LETTER LONG S uppercases
-- to ASCII `S`, so a Unicode fold matched this as a `SET` and pglet executed it
-- silently. Measured: 42601 on both engines.
-- @test
-- @expect-error: syntax error at or near "ſET"
ſET client_min_messages = 'warning';
