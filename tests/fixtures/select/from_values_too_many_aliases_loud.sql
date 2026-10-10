-- @setup

-- @test
-- @expect-error: more column aliases than the VALUES provides (42P10)
SELECT * FROM (VALUES (1, 2)) AS t(a, b, c);
