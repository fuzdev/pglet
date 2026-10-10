-- The skip reaches an action pglet refuses too: `RENAME` is `0A000` over a table that
-- exists, and over a name no relation holds it is the skip, as on 18.6.
-- @test
ALTER TABLE IF EXISTS nosuch_atie16 RENAME TO other_atie16;
