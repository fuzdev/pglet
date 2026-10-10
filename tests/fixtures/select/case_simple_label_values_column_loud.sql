-- A simple `CASE` label that is no integer, over a `VALUES` list's `integer` column, is
-- the label's own conversion error, as on 18.6.
-- @expect-error: invalid input syntax for type integer
-- @test
SELECT CASE v.x WHEN 'a' THEN 'one' ELSE 'other' END FROM (VALUES (1), (2)) v(x);
