-- @setup

-- @test
SELECT x FROM unnest(ARRAY['a', 'b', 'c']) AS t(x) ORDER BY x;
