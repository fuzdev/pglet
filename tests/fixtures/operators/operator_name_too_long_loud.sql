-- `scan.l`'s `{operator}` rule ends with `if (nchars >= NAMEDATALEN) yyerror("operator
-- too long")`, over the length left after the comment cut and the trailing-sign trim.
-- So 64 characters is a scanner error on both engines; a lexer that carried the name
-- on to analysis would answer the deferred `42883` instead.
-- @test
-- @expect-error: operator too long at 64 characters (42601)
SELECT 1 **************************************************************** 1;
