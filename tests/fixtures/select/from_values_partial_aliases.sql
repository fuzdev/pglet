-- @setup

-- @test
-- fewer aliases than columns is allowed: the unnamed trailing column keeps its
-- `column3` default
SELECT * FROM (VALUES (10, 20, 30), (40, 50, 60)) AS t(a, b) ORDER BY a;
