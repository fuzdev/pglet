-- One block takes a key shared, then exclusive, then shared again: its own holds never
-- conflict, so each call answers `void` at once.
-- @test
SELECT pg_advisory_xact_lock_shared(7);
SELECT pg_advisory_xact_lock(7);
SELECT pg_advisory_xact_lock_shared(7) AS again;
