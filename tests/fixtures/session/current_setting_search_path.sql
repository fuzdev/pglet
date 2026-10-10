-- `current_setting` reads the same table `SHOW <name>` answers from, so this is the
-- `SHOW search_path` value under the function spelling. It is fixed here
-- (`EDGES.md` row 66) and merely happens to equal 18.6's default.
-- @test
SELECT current_setting('search_path');
