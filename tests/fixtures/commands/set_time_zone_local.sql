-- `gram.y`'s `zone_value: LOCAL` is `DEFAULT` — a reset, not a zone named `local` (which 18.6
-- refuses `22023` through the generic spelling). The statement alone, for the reason
-- `set_time_zone_default.sql` gives.
-- @test
SET TIME ZONE LOCAL;
