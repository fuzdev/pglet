-- `current_setting` is strict in **both** arguments and `current_schemas` in its
-- one, so a NULL argument answers NULL rather than reaching the lookup — which is
-- what keeps `current_setting(NULL)` from being the `42704` its literal-name
-- sibling raises.
-- @test
SELECT
  current_setting(NULL) AS name_null,
  current_setting('search_path', NULL) AS missing_ok_null,
  current_schemas(NULL) AS implicit_null;
