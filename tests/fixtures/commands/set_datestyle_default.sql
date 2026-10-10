-- `SET <name> TO DEFAULT` on a parameter whose value pglet holds fixed is a reset, which
-- restores the value it has.
-- @test
SET DateStyle TO DEFAULT;
SHOW DateStyle;
