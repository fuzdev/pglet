-- An unrecognized SHOW parameter errors (Postgres 42704), instead of the old
-- `.contains()` dispatch that silently returned an empty result.
-- @test
-- @expect-error: unrecognized configuration parameter
SHOW bogus_setting;
