-- `SET NAMES` takes a string constant, `DEFAULT` or nothing (`gram.y`'s `opt_encoding`): an
-- unquoted word is 18.6's `42601` at it, not an encoding name. (The message, `syntax error at or
-- near "utf8"`, is compared by `catalog::commands`' `set_names_answers_as_the_grammar_does`.)
-- @test
-- @expect-error: syntax error at or near "utf8"
SET NAMES utf8;
