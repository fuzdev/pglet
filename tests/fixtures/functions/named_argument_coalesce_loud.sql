-- `COALESCE` is grammar, taking an expression list: a named argument is a syntax error.
-- @expect-error: syntax error at or near "=>"
-- @test
SELECT coalesce(a => 1);
