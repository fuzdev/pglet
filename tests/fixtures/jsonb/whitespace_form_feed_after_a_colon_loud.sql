-- 18.6's `json_lex` skips exactly space, tab, LF and CR ahead of a token, so a form feed
-- standing where a token starts is the start of a token and the input is invalid. Rust's
-- `str::trim` strips every Unicode `White_Space` character, which took this one away.
-- @expect-error: a form feed where a token starts
-- @expect-message
-- @test
SELECT (E'{"k":\x0c1}')::jsonb;
