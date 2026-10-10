-- The polarity control for the element-modifier rule: an array whose element
-- declares **no** modifier keeps every value whole. A cap invented here would read as
-- the over-correction the refusal half invites.
-- @test
SELECT '{abcdefg,xy}'::varchar[] AS v, '{1.25,2.5}'::numeric[] AS n, '{1,2}'::int4[] AS i;
