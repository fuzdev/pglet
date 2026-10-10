-- `SHOW` reads a quoted name case-insensitively too, as 18.6 does: `SHOW "Search_Path"` is
-- `SHOW search_path`.
-- @test
SHOW "Search_Path";
