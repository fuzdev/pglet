-- @setup

-- @test
-- @expect-error: VALUES rows of differing widths (42601)
SELECT * FROM (VALUES (1, 2), (3)) AS t;
