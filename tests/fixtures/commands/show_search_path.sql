-- pglet reports 18.6's own default, byte for byte. `$user` names a schema a fresh
-- database does not have on either engine, so both resolve this list to `public`
-- alone — the string is not borrowed, it is the same fact.
--
-- It used to answer `public`, which was a second spelling of the same resolution and
-- a divergence for no gain. The value does not move: every list pglet accepts
-- resolves the same way, so there is no session cell for it (`EDGES.md`).
-- @test
SHOW search_path;
