-- No `quote_ident(integer)`: `42883`.
-- @test
-- @expect-error: function quote_ident(integer) does not exist
-- @expect-message
SELECT quote_ident(1);
