-- The control: a line feed ends a `--` comment on both engines. The file spells it
-- `\r\n`, but the harness reads a fixture by lines, which drops that `\r`, so the line
-- feed is what is sent; `lexer::tests`' `a_line_comment_ends_at_a_carriage_return`
-- is the `\r\n` guard.
-- @test
SELECT 1 --c
+ 1 AS v;
