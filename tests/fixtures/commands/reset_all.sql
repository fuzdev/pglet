-- `RESET ALL` is the other half of a pool's check-in pair and was a bare `42601`
-- here for the same reason `DISCARD` was. It succeeds on both engines and reports
-- command tag `RESET` — not `RESET ALL`, measured on 18.6.
--
-- Every GUC pglet models is a fixed value it never moves, so restoring one to its
-- default is honestly a no-op; the shape check is not (`RESET` alone and
-- `RESET ALL x` are `42601`).
-- @test
RESET ALL;
