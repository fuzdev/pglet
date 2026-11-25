-- A type name's call takes no named argument either.
-- @expect-error: syntax error at or near "=>"
-- @test
SELECT interval(a => 1);
