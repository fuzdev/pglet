-- A parameter the startup `ParameterStatus` block already asserts, answered by
-- `SHOW` from the same table. The connection used to assert `on` at startup and
-- answer `42704 unrecognized configuration parameter` to the statement that asks
-- for it — one connection disagreeing with itself about a value a driver reads to
-- decide how it escapes string literals.
-- @test
SHOW standard_conforming_strings;
