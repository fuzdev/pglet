-- @setup

-- @test
-- an untyped bare NULL row adopts the other rows' concrete type (integer here) rather
-- than forcing the column to text
SELECT x FROM (VALUES (1), (NULL), (3)) AS t(x) ORDER BY x NULLS LAST;
