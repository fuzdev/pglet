-- A GUC name is case-insensitive on 18.6, and `current_setting` is not an exception.
-- Aliased so the two columns do not collide on the shared `current_setting` name.
-- @test
SELECT current_setting('SEARCH_PATH') AS shouty, current_setting('Search_Path') AS mixed;
