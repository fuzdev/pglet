-- `=>` is `scan.l`'s own token, not an operator and not `=` then `>`: `gram.y` takes it
-- only in a named function argument, so this is a grammar error on both engines.
-- @test
-- @expect-error: syntax error at or near "=>"
SELECT 1 => 2;
