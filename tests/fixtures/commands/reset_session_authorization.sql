-- The `reset_rest` spelling a connection pool actually sends on check-in, beside
-- `DISCARD ALL` and `RESET ALL`. It was `42601 syntax error at or near
-- "AUTHORIZATION"` here.
-- @test
RESET SESSION AUTHORIZATION;
