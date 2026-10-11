-- `RESET <name>` restores one setting. `search_path` is the one whose default
-- pglet can truthfully claim to restore: `"$user", public` resolves to the one schema
-- it has.
-- @test
RESET search_path;
