-- `DEALLOCATE ALL` on a session holding nothing succeeds and reports command tag
-- `DEALLOCATE ALL` — Postgres's `DropAllPreparedStatements` early-returns on a
-- hash table that was never created rather than raising. Session-hygiene statements
-- like this are what a pool sends on connection release, so the whole value of the
-- statement is that it never errors.
-- @test
DEALLOCATE ALL;
