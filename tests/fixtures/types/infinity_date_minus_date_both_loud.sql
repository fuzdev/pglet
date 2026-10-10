-- Both operands infinite, of opposite signs, is the same refusal.
-- @expect-error: cannot subtract infinite dates
-- @expect-message
-- @test
SELECT date '-infinity' - date 'infinity';
