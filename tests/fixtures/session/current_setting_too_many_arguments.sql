-- @expect-error: no three-argument current_setting signature
-- @test
SELECT current_setting('search_path', true, false);
