-- ...and the raw character stands first here. A gate run over the whole string body
-- ahead of the escape decoding would answer this row's code for both.
-- @expect-error: the raw control character, which stands first
-- @expect-message
-- @test
SELECT (E'"a\tb\\u0000"')::jsonb;
