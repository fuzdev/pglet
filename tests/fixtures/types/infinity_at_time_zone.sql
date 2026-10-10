-- `AT TIME ZONE` keeps an infinity.
-- @test
SELECT timestamptz '-infinity' AT TIME ZONE 'UTC' AS b;
